//! Print the roxygen Rd projection (`project_to_rd`) of an R file, or stdin
//! when no path is given — the projector-parity debugging loop's dump tool:
//!
//! ```text
//! cargo run --example rdproj -- file.R
//! diff <(cargo run -q --example rdproj -- case.R) case.rdtree
//! ```

use std::io::Read;

fn main() {
    let mut args = std::env::args().skip(1);
    let text = match args.next() {
        Some(path) => std::fs::read_to_string(path).expect("read file"),
        None => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .expect("read stdin");
            buf
        }
    };
    println!("{}", arity::roxygen::project_rd::project_to_rd(&text));
}
