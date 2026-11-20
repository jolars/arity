//! Resolve named method bodies from their registration's execution sequence.

use super::*;

type CallableOrigin = (PathBuf, usize);

enum BindingWrite<'a> {
    Callable(usize),
    Alias(&'a str),
    Unknown,
}

impl ClassIndex {
    pub(crate) fn registration_callable_origin(
        &self,
        id: &OopId,
        source: &MethodImplementationSource,
    ) -> Option<CallableOrigin> {
        let name = match source {
            MethodImplementationSource::Binding { name, owner } if owner.0.is_empty() => name,
            MethodImplementationSource::Reference { name } => name,
            MethodImplementationSource::Qualified { package, name } => {
                return self.external_callable_origin(&id.path, name, Some(package));
            }
            _ => return None,
        };
        let file = self.file(&id.path)?;
        let event = file
            .facts
            .oop
            .events
            .iter()
            .position(|event| matches!(event, OopEvent::Method(key) if *key == id.key))?;
        let execution = self.execution(&id.path);
        if !execution.complete || !self.class_file_complete(&id.path) {
            return None;
        }
        let events = &execution.events;
        let mut positions = events
            .iter()
            .enumerate()
            .filter_map(|(step, (path, index))| {
                (path == &id.path && *index == event).then_some(step)
            });
        let first = positions.next()?;
        // A deferred registration can run after a subsequent write. Likewise,
        // sourcing the same file twice can execute a site with different values.
        if !id.key.owner.0.is_empty()
            && events[first + 1..]
                .iter()
                .any(|(path, event)| self.binding_write(path, *event, name).is_some())
        {
            return None;
        }
        let origin_at = |step| {
            self.callable_before(name, &events[..step], |name| {
                self.external_callable_origin(&id.path, name, None)
            })
        };
        let origin = origin_at(first)?;
        positions
            .all(|step| origin_at(step).as_ref() == Some(&origin))
            .then_some(origin)
    }

    pub(crate) fn namespace_callable_origin(
        &self,
        root: &Path,
        name: &str,
    ) -> Option<CallableOrigin> {
        let file = self
            .oop_files()
            .find(|file| file.package_root.as_deref() == Some(root))?;
        let execution = self.execution(&file.path);
        if !execution.complete || !self.class_file_complete(&file.path) {
            return None;
        }
        self.callable_before(name, &execution.events, |_| None)
    }

    fn external_callable_origin(
        &self,
        path: &Path,
        name: &str,
        package: Option<&str>,
    ) -> Option<CallableOrigin> {
        let files = if package.is_some() {
            self.implementation_files(path, name, package)
        } else {
            self.imported_generic_files(path, name)
        };
        let mut roots = files
            .iter()
            .map(|file| self.file(file)?.package_root.as_deref())
            .collect::<Option<BTreeSet<_>>>()?
            .into_iter();
        let root = roots.next()?;
        if roots.next().is_some() {
            return None;
        }
        self.namespace_callable_origin(root, name)
    }

    fn callable_before<'a>(
        &'a self,
        mut name: &'a str,
        events: &[(PathBuf, usize)],
        external: impl FnOnce(&str) -> Option<CallableOrigin>,
    ) -> Option<CallableOrigin> {
        for (path, event) in events.iter().rev() {
            match self.binding_write(path, *event, name) {
                Some(BindingWrite::Callable(origin)) => return Some((path.clone(), origin)),
                Some(BindingWrite::Alias(target)) => name = target,
                Some(BindingWrite::Unknown) => return None,
                None => {}
            }
        }
        external(name)
    }

    fn binding_write(&self, path: &Path, event: usize, name: &str) -> Option<BindingWrite<'_>> {
        let file = self.file(path)?;
        match &file.facts.oop.events[event] {
            OopEvent::Callable(index) if file.facts.oop.callables[*index].name == name => Some(
                BindingWrite::Callable(file.facts.oop.callables[*index].origin),
            ),
            OopEvent::Generator {
                name: binding,
                target,
            } if binding == name => match target {
                GeneratorRef::Free(target) => Some(BindingWrite::Alias(target)),
                _ => Some(BindingWrite::Unknown),
            },
            OopEvent::Generic {
                name: ClassName::Known(binding),
                target,
            } if binding == name && target.owner.0.is_empty() => Some(BindingWrite::Unknown),
            OopEvent::Method(key)
                if key.system == OopSystem::S4
                    && key.owner.0.is_empty()
                    && file.facts.oop.methods.iter().any(|method| {
                        method.key == *key && method.generic.as_ref().is_some_and(|generic| {
                            matches!(&generic.name, ClassName::Known(binding) if binding == name)
                                && !generic.by_binding
                                && generic.package.is_none()
                        })
                    }) =>
            {
                Some(BindingWrite::Unknown)
            }
            OopEvent::UncertainGenerator { name: binding, .. } | OopEvent::Binding(binding)
                if binding == name =>
            {
                Some(BindingWrite::Unknown)
            }
            OopEvent::InvalidateBindings => Some(BindingWrite::Unknown),
            _ => None,
        }
    }
}
