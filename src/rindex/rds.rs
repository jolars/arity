//! A minimal reader for R's RDS serialization format.
//!
//! Scope: enough of the format to read the `.rds` index files and lazy-load
//! databases that ship inside an installed package — named lists of integer
//! offsets (`.rdx`), the help-topic data frame (`Meta/Rd.rds`), and (for later
//! phases) the closures stored in `.rdb`. It is **not** a general-purpose R
//! deserializer; types we never expect are parsed structurally where their
//! layout is known and rejected otherwise (we cannot skip an unknown type
//! whose length we don't know).
//!
//! Format facts this relies on (verified against R 4.5.3 output):
//! - Streams begin with `X\n` (XDR / big-endian binary). Other encodings are
//!   rejected.
//! - Files (`.rds`/`.rdx`) are gzip-wrapped; [`read_rds`] gunzips transparently.
//! - Integers and reals are big-endian; `NA` is `i32::MIN`.

use std::io::Read;

use smol_str::SmolStr;

/// A decoded R object plus its attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct Robj {
    pub kind: Rkind,
    /// Attribute name → value (e.g. `"names"`, `"class"`, `"row.names"`).
    pub attr: Vec<(SmolStr, Robj)>,
}

/// The decoded payload of an R object, limited to the kinds this reader needs.
#[derive(Debug, Clone, PartialEq)]
pub enum Rkind {
    /// `NULL` / empty pairlist tail.
    Null,
    /// The empty-argument placeholder (`MISSINGARG_SXP`) — a formal with no
    /// default.
    Missing,
    Logical(Vec<Option<bool>>),
    Int(Vec<Option<i32>>),
    Real(Vec<f64>),
    /// Character vector; `None` is `NA_character_`.
    Str(Vec<Option<String>>),
    /// Generic vector (`VECSXP`).
    List(Vec<Robj>),
    /// Pairlist or language object (`LISTSXP`/`LANGSXP`), flattened to its
    /// elements; each carries an optional tag (the formal/argument name).
    Pairlist(Vec<PairlistItem>),
    /// A symbol; carries its name.
    Symbol(SmolStr),
    /// A closure: formals (a pairlist) and body. The body is read structurally
    /// but not interpreted.
    Closure {
        formals: Box<Robj>,
        body: Box<Robj>,
    },
    /// An environment — contents not retained (placeholder so refs resolve).
    Env,
    /// A builtin or special primitive (e.g. an export aliased to `` `+` ``).
    /// Callable, but has no R-level formals.
    Builtin,
    /// A type whose value we don't model but whose bytes were consumed
    /// correctly (S4 objects, external pointers, promises, complex/raw vectors).
    Opaque,
}

/// One element of a pairlist / argument list.
#[derive(Debug, Clone, PartialEq)]
pub struct PairlistItem {
    pub tag: Option<SmolStr>,
    pub value: Robj,
}

#[derive(Debug)]
pub enum RdsError {
    UnexpectedEof,
    BadHeader(String),
    UnsupportedType(u8),
    BadRef(i32),
    Decompress(String),
    Utf8,
}

impl std::fmt::Display for RdsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RdsError::UnexpectedEof => write!(f, "unexpected end of RDS stream"),
            RdsError::BadHeader(s) => write!(f, "bad RDS header: {s}"),
            RdsError::UnsupportedType(t) => write!(f, "unsupported SEXP type {t}"),
            RdsError::BadRef(i) => write!(f, "bad reference index {i}"),
            RdsError::Decompress(s) => write!(f, "decompression failed: {s}"),
            RdsError::Utf8 => write!(f, "invalid UTF-8 in RDS string"),
        }
    }
}

impl std::error::Error for RdsError {}

type Result<T> = std::result::Result<T, RdsError>;

// SEXP type tags we recognize.
const NILSXP: u8 = 0;
const SYMSXP: u8 = 1;
const LISTSXP: u8 = 2;
const CLOSXP: u8 = 3;
const ENVSXP: u8 = 4;
const PROMSXP: u8 = 5;
const LANGSXP: u8 = 6;
const SPECIALSXP: u8 = 7;
const BUILTINSXP: u8 = 8;
const CHARSXP: u8 = 9;
const LGLSXP: u8 = 10;
const INTSXP: u8 = 13;
const REALSXP: u8 = 14;
const CPLXSXP: u8 = 15;
const STRSXP: u8 = 16;
const DOTSXP: u8 = 17;
const VECSXP: u8 = 19;
const EXPRSXP: u8 = 20;
const BCODESXP: u8 = 21;
const EXTPTRSXP: u8 = 22;
const RAWSXP: u8 = 24;
const S4SXP: u8 = 25;

// Pseudo-types used only in serialization.
const REFSXP: u8 = 255;
const NILVALUE_SXP: u8 = 254;
const GLOBALENV_SXP: u8 = 253;
const UNBOUNDVALUE_SXP: u8 = 252;
const MISSINGARG_SXP: u8 = 251;
const BASENAMESPACE_SXP: u8 = 250;
const NAMESPACESXP: u8 = 249;
const PACKAGESXP: u8 = 248;
const PERSISTSXP: u8 = 247;
const BCREPDEF: u8 = 244;
const BCREPREF: u8 = 243;
const EMPTYENV_SXP: u8 = 242;
const BASEENV_SXP: u8 = 241;
const ATTRLANGSXP: u8 = 240;
const ATTRLISTSXP: u8 = 239;
const ALTREP_SXP: u8 = 238;

const NA_INT: i32 = i32::MIN;

/// Read an RDS document from raw file bytes, transparently gunzipping a
/// gzip-wrapped file (`.rds`/`.rdx`).
pub fn read_rds(bytes: &[u8]) -> Result<Robj> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut decoder = flate2::read::GzDecoder::new(bytes);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|e| RdsError::Decompress(e.to_string()))?;
        read_rds_stream(&out)
    } else {
        read_rds_stream(bytes)
    }
}

/// Read an uncompressed RDS stream (header + one top-level object). Used for
/// already-decompressed lazy-load blobs.
pub fn read_rds_stream(bytes: &[u8]) -> Result<Robj> {
    let mut r = Reader::new(bytes);
    r.read_header()?;
    r.read_item()
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    /// Reference table (1-based on the wire); holds symbols, environments, and
    /// persistent strings as they are first seen.
    refs: Vec<Robj>,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader {
            buf,
            pos: 0,
            refs: Vec::new(),
        }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(RdsError::UnexpectedEof)?;
        let slice = self.buf.get(self.pos..end).ok_or(RdsError::UnexpectedEof)?;
        self.pos = end;
        Ok(slice)
    }

    fn read_i32(&mut self) -> Result<i32> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_f64(&mut self) -> Result<f64> {
        let b = self.take(8)?;
        Ok(f64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn read_length(&mut self) -> Result<usize> {
        let len = self.read_i32()?;
        if len == -1 {
            // Long vector: two more i32 encode a 64-bit length.
            let hi = self.read_i32()? as i64;
            let lo = self.read_i32()? as i64;
            Ok(((hi << 32) | (lo & 0xffff_ffff)) as usize)
        } else if len < 0 {
            Err(RdsError::BadHeader(format!("negative length {len}")))
        } else {
            Ok(len as usize)
        }
    }

    fn read_header(&mut self) -> Result<()> {
        let magic = self.take(2)?;
        match magic {
            b"X\n" => {}
            b"B\n" | b"A\n" => {
                return Err(RdsError::BadHeader(
                    "only XDR (big-endian) RDS is supported".to_string(),
                ));
            }
            other => return Err(RdsError::BadHeader(format!("magic {other:?}"))),
        }
        let version = self.read_i32()?;
        let _writer_version = self.read_i32()?;
        let _min_reader_version = self.read_i32()?;
        if version == 3 {
            // Native-encoding string (e.g. "UTF-8"); we ignore its content.
            let n = self.read_length()?;
            let _ = self.take(n)?;
        } else if version != 2 {
            return Err(RdsError::BadHeader(format!(
                "unsupported serialization version {version}"
            )));
        }
        Ok(())
    }

    /// Read a CHARSXP body (length-prefixed bytes) honoring the encoding bits.
    fn read_charsxp(&mut self, gp: u32) -> Result<Option<String>> {
        let len = self.read_i32()?;
        if len == -1 {
            return Ok(None); // NA_character_
        }
        let bytes = self.take(len as usize)?;
        // gp encoding flags: bit 2 = Latin-1, bit 3 = UTF-8, bit 6 = ASCII.
        let is_latin1 = gp & (1 << 2) != 0;
        let s = if is_latin1 {
            // Latin-1: each byte is a Unicode code point.
            bytes.iter().map(|&b| b as char).collect()
        } else {
            // UTF-8, ASCII, or native — treat as UTF-8 (lossy-free for our
            // inputs); fall back to lossy only if invalid.
            match std::str::from_utf8(bytes) {
                Ok(s) => s.to_string(),
                Err(_) => String::from_utf8_lossy(bytes).into_owned(),
            }
        };
        Ok(Some(s))
    }

    fn read_item(&mut self) -> Result<Robj> {
        let flags = self.read_i32()?;
        let ty = (flags & 0xff) as u8;
        let has_attr = (flags >> 9) & 1 == 1;
        let has_tag = (flags >> 10) & 1 == 1;
        let gp = (flags >> 12) as u32;

        match ty {
            NILVALUE_SXP | NILSXP => Ok(Robj::null()),
            MISSINGARG_SXP => Ok(Robj::bare(Rkind::Missing)),
            UNBOUNDVALUE_SXP => Ok(Robj::bare(Rkind::Opaque)),
            GLOBALENV_SXP | EMPTYENV_SXP | BASEENV_SXP | BASENAMESPACE_SXP => {
                Ok(Robj::bare(Rkind::Env))
            }
            REFSXP => {
                let idx = self.ref_index(flags)?;
                self.refs
                    .get(idx)
                    .cloned()
                    .ok_or(RdsError::BadRef(idx as i32))
            }
            SYMSXP => {
                let name = self.read_item()?; // a CHARSXP
                let sym = Robj::bare(Rkind::Symbol(name.as_str().unwrap_or_default().into()));
                self.refs.push(sym.clone());
                Ok(sym)
            }
            CHARSXP => {
                let s = self.read_charsxp(gp)?;
                Ok(Robj::bare(Rkind::Str(vec![s])))
            }
            LGLSXP => {
                let n = self.read_length()?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    let x = self.read_i32()?;
                    v.push(if x == NA_INT { None } else { Some(x != 0) });
                }
                self.finish(Rkind::Logical(v), has_attr)
            }
            INTSXP => {
                let n = self.read_length()?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    let x = self.read_i32()?;
                    v.push(if x == NA_INT { None } else { Some(x) });
                }
                self.finish(Rkind::Int(v), has_attr)
            }
            REALSXP => {
                let n = self.read_length()?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(self.read_f64()?);
                }
                self.finish(Rkind::Real(v), has_attr)
            }
            CPLXSXP => {
                let n = self.read_length()?;
                let _ = self.take(n * 16)?; // two f64 per element
                self.finish(Rkind::Opaque, has_attr)
            }
            RAWSXP => {
                let n = self.read_length()?;
                let _ = self.take(n)?;
                self.finish(Rkind::Opaque, has_attr)
            }
            STRSXP => {
                let n = self.read_length()?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    let item = self.read_item()?; // CHARSXP (or REF)
                    v.push(item.into_single_str());
                }
                self.finish(Rkind::Str(v), has_attr)
            }
            VECSXP | EXPRSXP => {
                let n = self.read_length()?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(self.read_item()?);
                }
                self.finish(Rkind::List(v), has_attr)
            }
            LISTSXP | LANGSXP | DOTSXP | ATTRLISTSXP | ATTRLANGSXP => {
                self.read_pairlist(has_attr, has_tag)
            }
            CLOSXP => {
                // A closure is a cons cell: [attrib?] [tag=CLOENV if has_tag]
                // CAR=formals CDR=body.
                let attr = if has_attr {
                    self.read_attributes()?
                } else {
                    Vec::new()
                };
                if has_tag {
                    let _cloenv = self.read_item()?;
                }
                let formals = self.read_item()?;
                let body = self.read_item()?;
                Ok(Robj {
                    kind: Rkind::Closure {
                        formals: Box::new(formals),
                        body: Box::new(body),
                    },
                    attr,
                })
            }
            ENVSXP => self.read_env(),
            PROMSXP => {
                // [attrib?] value, expr, env  (order: value, tag(expr), env)
                if has_attr {
                    let _ = self.read_attributes()?;
                }
                let _ = self.read_item()?;
                let _ = self.read_item()?;
                let _ = self.read_item()?;
                Ok(Robj::bare(Rkind::Opaque))
            }
            BUILTINSXP | SPECIALSXP => {
                let n = self.read_i32()?; // name length
                let _ = self.take(n as usize)?;
                Ok(Robj::bare(Rkind::Builtin))
            }
            BCODESXP => self.read_bytecode(),
            S4SXP => {
                // S4 object: attributes only.
                let attr = if has_attr {
                    self.read_attributes()?
                } else {
                    Vec::new()
                };
                Ok(Robj {
                    kind: Rkind::Opaque,
                    attr,
                })
            }
            EXTPTRSXP => {
                let _ = self.read_item()?; // prot
                let _ = self.read_item()?; // tag
                Ok(Robj::bare(Rkind::Opaque))
            }
            ALTREP_SXP => self.read_altrep(),
            NAMESPACESXP | PACKAGESXP | PERSISTSXP => {
                // A string vector naming the namespace/package (R's
                // `InStringVec`): an `i32` "has-names" flag (always 0), an
                // `i32` length, then that many CHARSXP elements. Stored in the
                // ref table so later REFSXPs resolve.
                let _has_names = self.read_i32()?;
                let len = self.read_length()?;
                let mut v = Vec::with_capacity(len);
                for _ in 0..len {
                    let item = self.read_item()?; // CHARSXP
                    v.push(item.into_single_str());
                }
                let names = Robj::bare(Rkind::Str(v));
                self.refs.push(names.clone());
                Ok(names)
            }
            BCREPDEF | BCREPREF => Err(RdsError::UnsupportedType(ty)),
            other => Err(RdsError::UnsupportedType(other)),
        }
    }

    fn ref_index(&mut self, flags: i32) -> Result<usize> {
        let packed = flags >> 8;
        let one_based = if packed == 0 {
            self.read_i32()?
        } else {
            packed
        };
        if one_based < 1 {
            return Err(RdsError::BadRef(one_based));
        }
        Ok((one_based - 1) as usize)
    }

    /// Read an attribute pairlist into `name → value` pairs.
    fn read_attributes(&mut self) -> Result<Vec<(SmolStr, Robj)>> {
        let attrs = self.read_item()?;
        Ok(match attrs.kind {
            Rkind::Pairlist(items) => items
                .into_iter()
                .filter_map(|it| it.tag.map(|t| (t, it.value)))
                .collect(),
            _ => Vec::new(),
        })
    }

    /// Attach attributes (if present) to a freshly read vector value.
    fn finish(&mut self, kind: Rkind, has_attr: bool) -> Result<Robj> {
        let attr = if has_attr {
            self.read_attributes()?
        } else {
            Vec::new()
        };
        Ok(Robj { kind, attr })
    }

    fn read_pairlist(&mut self, mut has_attr: bool, mut has_tag: bool) -> Result<Robj> {
        let mut items = Vec::new();
        let mut head_attr = Vec::new();
        let mut first = true;
        loop {
            // Each cons cell: [attrib?] [tag?] CAR CDR.
            let attr = if has_attr {
                self.read_attributes()?
            } else {
                Vec::new()
            };
            if first {
                head_attr = attr;
                first = false;
            }
            let tag = if has_tag {
                let t = self.read_item()?;
                t.symbol_name()
            } else {
                None
            };
            let car = self.read_item()?;
            items.push(PairlistItem { tag, value: car });

            // Read CDR's flags to decide whether to continue.
            let flags = self.read_i32()?;
            let ty = (flags & 0xff) as u8;
            if ty == NILVALUE_SXP || ty == NILSXP {
                break;
            }
            if ty == REFSXP {
                // A shared tail — resolve and stop (rare for our inputs).
                let idx = self.ref_index(flags)?;
                let _ = self
                    .refs
                    .get(idx)
                    .cloned()
                    .ok_or(RdsError::BadRef(idx as i32))?;
                break;
            }
            if !matches!(ty, LISTSXP | LANGSXP | DOTSXP | ATTRLISTSXP | ATTRLANGSXP) {
                return Err(RdsError::BadHeader(format!(
                    "unexpected pairlist tail type {ty}"
                )));
            }
            has_attr = (flags >> 9) & 1 == 1;
            has_tag = (flags >> 10) & 1 == 1;
        }
        Ok(Robj {
            kind: Rkind::Pairlist(items),
            attr: head_attr,
        })
    }

    fn read_env(&mut self) -> Result<Robj> {
        // Reserve the ref slot first (environments can be self-referential).
        let slot = self.refs.len();
        self.refs.push(Robj::bare(Rkind::Env));
        let _locked = self.read_i32()?;
        let _enclos = self.read_item()?;
        let _frame = self.read_item()?;
        let _hashtab = self.read_item()?;
        let _attrib = self.read_item()?;
        let env = Robj::bare(Rkind::Env);
        self.refs[slot] = env.clone();
        Ok(env)
    }

    /// Consume a `BCODESXP`. We never interpret bytecode — we only walk its
    /// bytes so the surrounding stream stays aligned (the body of a fetched
    /// closure is left `Opaque`; its formals were already read). This is a
    /// faithful port of R's `serialize.c` `ReadBC`/`ReadBC1`/`ReadBCConsts`/
    /// `ReadBCLang`. The crux: inside this sub-grammar, type tags are **bare
    /// `read_i32` values**, not the packed `flags` that [`read_item`] decodes.
    fn read_bytecode(&mut self) -> Result<Robj> {
        let nreps = self.read_i32()?;
        // The reps table lets a BCREPREF point back at an earlier BCREPDEF. We
        // only consume bytes, so placeholder nulls suffice; clamp the count to
        // guard against a corrupt/negative length.
        let mut reps: Vec<Robj> = vec![Robj::null(); nreps.max(0) as usize];
        self.read_bc1(&mut reps)
    }

    /// `ReadBC1`: the code object (an INTSXP) followed by the constant pool.
    fn read_bc1(&mut self, reps: &mut Vec<Robj>) -> Result<Robj> {
        let _code = self.read_item()?;
        let nconsts = self.read_i32()?;
        for _ in 0..nconsts {
            let ty = self.read_i32()? as u8; // bare type tag
            match ty {
                BCODESXP => {
                    self.read_bc1(reps)?;
                }
                LANGSXP | LISTSXP | ATTRLANGSXP | ATTRLISTSXP | BCREPDEF | BCREPREF => {
                    self.read_bc_lang(ty, reps)?;
                }
                _ => {
                    // Any other constant is an ordinary packed-flags item.
                    self.read_item()?;
                }
            }
        }
        Ok(Robj::bare(Rkind::Opaque))
    }

    /// `ReadBCLang`: dispatch a bare type tag within the bytecode const pool.
    fn read_bc_lang(&mut self, ty: u8, reps: &mut Vec<Robj>) -> Result<Robj> {
        match ty {
            BCREPREF => {
                // A back-reference to a previously defined node: one index.
                let _idx = self.read_i32()?;
                Ok(Robj::null())
            }
            BCREPDEF => {
                // A definition slot, then the real type follows.
                let _pos = self.read_i32()?;
                let real_ty = self.read_i32()? as u8;
                self.read_bc_lang_node(real_ty, reps)
            }
            LANGSXP | LISTSXP | ATTRLANGSXP | ATTRLISTSXP => self.read_bc_lang_node(ty, reps),
            // Any other real type is an ordinary item.
            _ => self.read_item(),
        }
    }

    /// Read one `LANG`/`LIST` cons cell within the bytecode grammar: optional
    /// attributes (for the `ATTR*` variants), a tag, then CAR and CDR — each
    /// introduced by its own bare type tag.
    fn read_bc_lang_node(&mut self, ty: u8, reps: &mut Vec<Robj>) -> Result<Robj> {
        match ty {
            LANGSXP | LISTSXP | ATTRLANGSXP | ATTRLISTSXP => {
                if ty == ATTRLANGSXP || ty == ATTRLISTSXP {
                    let _attrib = self.read_item()?;
                }
                let _tag = self.read_item()?;
                let car_ty = self.read_i32()? as u8;
                let _car = self.read_bc_lang(car_ty, reps)?;
                let cdr_ty = self.read_i32()? as u8;
                let _cdr = self.read_bc_lang(cdr_ty, reps)?;
                Ok(Robj::bare(Rkind::Opaque))
            }
            _ => self.read_item(),
        }
    }

    fn read_altrep(&mut self) -> Result<Robj> {
        // ALTREP_SXP: class descriptor (a LISTSXP of [class-sym, pkg-sym,
        // type-int]), serialized state, and attributes.
        let class = self.read_item()?;
        let state = self.read_item()?;
        let _attr = self.read_item()?;
        let class_name = altrep_class_name(&class);
        match class_name.as_deref() {
            Some("compact_intseq") => Ok(expand_compact_intseq(&state)),
            Some("wrap_integer") | Some("wrap_real") | Some("wrap_logical")
            | Some("wrap_string") => {
                // State is a pairlist whose first element is the wrapped data.
                if let Rkind::Pairlist(items) = state.kind {
                    Ok(items
                        .into_iter()
                        .next()
                        .map(|it| it.value)
                        .unwrap_or_else(Robj::null))
                } else {
                    Ok(state)
                }
            }
            _ => Err(RdsError::UnsupportedType(ALTREP_SXP)),
        }
    }
}

fn altrep_class_name(class: &Robj) -> Option<String> {
    // class descriptor is a pairlist; the first element is the class symbol.
    if let Rkind::Pairlist(items) = &class.kind
        && let Some(first) = items.first()
        && let Rkind::Symbol(s) = &first.value.kind
    {
        return Some(s.to_string());
    }
    None
}

fn expand_compact_intseq(state: &Robj) -> Robj {
    // State is a REALSXP of [n, start, step].
    if let Rkind::Real(v) = &state.kind
        && v.len() == 3
    {
        let n = v[0] as usize;
        let start = v[1];
        let step = v[2];
        let seq = (0..n)
            .map(|i| Some((start + step * i as f64) as i32))
            .collect();
        return Robj::bare(Rkind::Int(seq));
    }
    Robj::null()
}

impl Robj {
    fn null() -> Robj {
        Robj {
            kind: Rkind::Null,
            attr: Vec::new(),
        }
    }

    fn bare(kind: Rkind) -> Robj {
        Robj {
            kind,
            attr: Vec::new(),
        }
    }

    fn symbol_name(&self) -> Option<SmolStr> {
        match &self.kind {
            Rkind::Symbol(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// For a length-1 character vector / CHARSXP, the single string value.
    fn into_single_str(self) -> Option<String> {
        match self.kind {
            Rkind::Str(mut v) if v.len() == 1 => v.pop().flatten(),
            _ => None,
        }
    }

    /// The first string element, if this is a (CHAR/STR) string vector.
    pub fn as_str(&self) -> Option<&str> {
        match &self.kind {
            Rkind::Str(v) => v.first().and_then(|s| s.as_deref()),
            _ => None,
        }
    }

    /// The full string vector, if this is a character vector.
    pub fn as_str_vec(&self) -> Option<&[Option<String>]> {
        match &self.kind {
            Rkind::Str(v) => Some(v),
            _ => None,
        }
    }

    /// The integer vector, if this is an integer vector.
    pub fn as_int_vec(&self) -> Option<&[Option<i32>]> {
        match &self.kind {
            Rkind::Int(v) => Some(v),
            _ => None,
        }
    }

    /// The elements, if this is a generic list (`VECSXP`).
    pub fn as_list(&self) -> Option<&[Robj]> {
        match &self.kind {
            Rkind::List(v) => Some(v),
            _ => None,
        }
    }

    /// The named attribute, if present.
    pub fn attr(&self, name: &str) -> Option<&Robj> {
        self.attr.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    /// The `names` attribute as a vector of `Option<&str>`.
    pub fn names(&self) -> Option<Vec<Option<&str>>> {
        self.attr("names").and_then(|n| match &n.kind {
            Rkind::Str(v) => Some(v.iter().map(|s| s.as_deref()).collect()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_xdr() {
        let err = read_rds_stream(b"B\n\0\0\0\x02").unwrap_err();
        assert!(matches!(err, RdsError::BadHeader(_)));
    }

    /// A namespace reference (`NAMESPACESXP`) is a length-prefixed string
    /// vector, not a single item. Decoding it as the closure's environment must
    /// leave the stream aligned so the formals that follow read correctly.
    #[test]
    fn decodes_namespace_string_vec() {
        // Hand-built XDR stream: header, then NAMESPACESXP { has_names=0,
        // len=2, "magrittr", "2.0.4" }.
        let mut s: Vec<u8> = Vec::new();
        s.extend_from_slice(b"X\n");
        s.extend_from_slice(&2i32.to_be_bytes()); // version
        s.extend_from_slice(&0x0003_0603i32.to_be_bytes()); // writer
        s.extend_from_slice(&0x0002_0300i32.to_be_bytes()); // min reader
        s.extend_from_slice(&(NAMESPACESXP as i32).to_be_bytes());
        s.extend_from_slice(&0i32.to_be_bytes()); // has_names
        s.extend_from_slice(&2i32.to_be_bytes()); // len
        for name in ["magrittr", "2.0.4"] {
            s.extend_from_slice(&(CHARSXP as i32).to_be_bytes());
            s.extend_from_slice(&(name.len() as i32).to_be_bytes());
            s.extend_from_slice(name.as_bytes());
        }
        let obj = read_rds_stream(&s).expect("decode namespace");
        assert_eq!(
            obj.as_str_vec(),
            Some([Some("magrittr".to_string()), Some("2.0.4".to_string())].as_slice())
        );
    }
}
