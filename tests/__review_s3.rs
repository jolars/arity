use arity::{
    ast::CallExpr,
    incremental::IncrementalDatabase,
    parser::parse,
    project::classes::{S3ClassEvidence, file_class_defs},
    semantic::SemanticModel,
};
use rowan::ast::AstNode as _;
use std::path::Path;
fn evidence(source: &str) -> S3ClassEvidence {
    let root = parse(source).cst;
    let model = SemanticModel::build(&root);
    let facts = file_class_defs(&model, &root);
    let path = Path::new("/s3.R");
    let index = IncrementalDatabase::default()
        .snapshot()
        .class_index_in(path, &facts, &root, &model);
    let value = root
        .descendants()
        .filter_map(CallExpr::cast)
        .find(|c| c.callee_name().as_deref() == Some("probe"))
        .unwrap()
        .arg_list()
        .unwrap()
        .args()
        .next()
        .unwrap()
        .value()
        .unwrap();
    index.s3_class_evidence(path, &root, &model, &value)
}
#[test]
fn explore() {
    let sources = [
        "x <- structure(list(), class='before'); probe(x)",
        "x <- structure(list(), class='before'); if (flag) { x <- structure(list(), class='other') }; probe(x)",
        "x <- structure(list(), class='before'); f <- function() class(x) <<- 'after'; f(); probe(x)",
        "x <- structure(list(), class='before'); f <- function() class(x) <<- 'after'; alias <- f; alias(); probe(x)",
        "x <- structure(list(), class='before'); f <- function() class(x) <<- 'after'; (f)(); probe(x)",
        "x <- structure(list(), class='before'); f <- function() class(x) <<- 'after'; do.call(f, list()); probe(x)",
        "x <- structure(list(), class='before'); mutator <- eval; mutator(quote(class(x) <- 'after')); probe(x)",
        "new_value <- function() { x <- structure(list(), class='before'); mutator <- eval; mutator(quote(class(x) <- 'after')); x }; probe(new_value())",
        "x <- structure(list(), class='before'); base::eval(quote(class(x) <- 'after')); probe(x)",
        "x <- structure(list(), class='before'); f <- function(arg) force(arg); f(class(x) <- 'after'); probe(x)",
        "x <- structure(list(), class='before'); f <- function(arg) force(arg); arg <- quote(class(x) <- 'after'); f(eval(arg)); probe(x)",
        "x <- structure(list(), class='before'); structure(list(), class='ignored', other=(x <- structure(list(), class='after'))); probe(x)",
        "new_value <- function() { x <- structure(list(), class='before'); y <- function() 1; y(); x }; probe(new_value())",
        "new_value <- function() { x <- structure(list(), class='before'); y <- function() class(x) <<- 'after'; y(); x }; probe(new_value())",
        "probe(structure(list(), class = { c <- function(...) 'wrong'; c('right') }))",
        "factory <- function() { structure <- base::structure; structure(list(), class='right') }; probe(factory())",
        "factory <- function() structure(list(), class='right'); factory <- NULL; probe(factory())",
        "x <- structure(list(), class='before'); probe(structure(x, class=class(x)))",
        "x <- structure(list(), class='before'); probe(structure(x, class=oldClass(x)))",
        "x <- structure(list(), class='before'); quote(x <- structure(list(), class='after')); probe(x)",
        "x <- structure(list(), class='before'); probe(list())",
        "x <- structure(list(), class='before'); probe(base::list())",
        "x <- structure(list(), class='before'); probe(unclass(x))",
        "probe(base::structure(.Data = list(), class = 'a'))",
        "x <- structure(list(), class='before'); if (FALSE) {} else class(x) <- 'after'; probe(x)",
        "new_value <- function() {x <- structure(list(), class='before'); base::`body<-`(new_value, quote(list())); x}; probe(new_value())",
        "x <- structure(list(), class='before'); f <- function() list(); class(f) <- 'foo'; probe(f())",
        "probe(structure(list(), class = 'foo', ...=NULL))",
        "probe(structure(list(), class = base::c('foo', 'bar')))",
        "probe(structure(1, class='foo', dim=NULL))",
    ];
    for (i, source) in sources.iter().enumerate() {
        println!("{i}: {:?}: {source}", evidence(source));
    }
}
#[test]
fn explore_limits_and_dependencies() {
    for count in [20, 30, 31, 32, 33, 40] {
        let before = "x <- structure(list(), class='before'); ";
        let nested = format!(
            "{}class(x) <- 'after'; probe(x){}",
            "{".repeat(count),
            "}".repeat(count)
        );
        let sequential = format!(
            "{}class(x) <- 'after'{}; probe(x)",
            "{".repeat(count),
            "}".repeat(count)
        );
        println!(
            "nested {count}: {:?}",
            evidence(&format!("{before}{nested}"))
        );
        println!(
            "sequential {count}: {:?}",
            evidence(&format!("{before}{sequential}"))
        );
    }
    use arity::{
        incremental::{QueryKind, file_class_defs as tracked_facts},
        project::project_classes,
    };
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/s3.R");
    let file = db.upsert_file(
        path,
        "new_value <- function() structure(list(), class='new'); unrelated <- function(x) x\n",
    );
    db.set_workspace_members(vec![file], vec![]);
    let _ = project_classes(&db);
    let old = tracked_facts(&db, file).clone();
    db.clear_query_log();
    db.set_file_text(file,"new_value <- function() structure(list(), class='new'); unrelated <- function(x) class(x)\n");
    let new = tracked_facts(&db, file).clone();
    let _ = project_classes(&db);
    println!("old_s3: {:?}, new_s3: {:?}", old.s3, new.s3);
    println!(
        "old_other_eq: {}",
        old.declarations == new.declarations
            && old.generators == new.generators
            && old.generator_aliases == new.generator_aliases
            && old.oop == new.oop
    );
    println!(
        "query_rebuilt: {}",
        db.query_log()
            .iter()
            .any(|q| q.kind == QueryKind::ProjectClasses)
    );
}
#[test]
fn explore_early_exits() {
    for source in [
        "new_value <- function() { return(structure(list(), class='early')) + 1; structure(list(), class='late') }; probe(new_value())",
        "new_value <- function() { -return(structure(list(), class='early')); structure(list(), class='late') }; probe(new_value())",
        "new_value <- function() { list()[return(structure(list(), class='early'))]; structure(list(), class='late') }; probe(new_value())",
        "ret <- return; new_value <- function() { list(ret(structure(list(), class='early'))); structure(list(), class='late') }; probe(new_value())",
        "ret <- return; new_value <- function() { c(ret(structure(list(), class='early'))); structure(list(), class='late') }; probe(new_value())",
        "dispatch <- UseMethod; new_value <- function(x) { dispatch('generic'); structure(list(), class='late') }; generic.a <- function(x) structure(list(), class='early'); probe(new_value(structure(list(), class='a')))",
        "x <- structure(list(), class='before'); expr <- quote(class(x) <- 'after'); mutator <- eval; mutator(expr); probe(x)",
        "expr <- quote(class(x) <- 'after'); new_value <- function() { x <- structure(list(), class='before'); mutator <- eval; mutator(expr); x }; probe(new_value())",
        "x <- structure(list(), class='before'); f <- function() class(x) <<- 'after'; fns <- list(f); fns[[1]](); probe(x)",
    ] {
        println!("{:?}: {source}", evidence(source));
    }
}
