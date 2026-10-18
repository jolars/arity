//! Warn about names that can be mistaken for an entirely ASCII spelling.
//!
//! A small, explicit map avoids treating all non-ASCII identifiers as typos.
//! Names containing other letters, including Greek statistical symbols, stay
//! untouched. Renaming can change binding and member lookup, so no fix is safe.

use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct ConfusableIdentifier;

const EXAMPLES: &[Example] = &[
    Example {
        caption: "Cyrillic `с` looks like Latin `c` but defines a different binding:",
        source: "с <- 1\n",
    },
    Example {
        caption: "A Cyrillic `а` can hide inside an otherwise ASCII name:",
        source: "pаyload <- 1\n",
    },
];

impl Rule for ConfusableIdentifier {
    fn id(&self) -> &'static str {
        "confusable-identifier"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn description(&self) -> &'static str {
        "Flag unquoted identifiers made entirely of ASCII characters and a \
         conservative set of Cyrillic lookalikes or fullwidth Latin letters. \
         Report the lookalikes' Unicode code points and the likely ASCII spelling. \
         R treats these spellings as different names.\n\n\
         Greek letters, accented Latin letters, and names containing other \
         non-Latin letters are left alone. Entirely Cyrillic names can still be \
         reported if every letter resembles ASCII. Strings, comments, roxygen, \
         and backtick names are ignored. This is a visual warning, not a ban on \
         non-Latin identifiers or a complete Unicode confusability check; see \
         [Unicode's confusability guidance](https://www.unicode.org/reports/tr39/). \
         No automatic fix is offered because the intended binding is unknown."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::IDENT]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(token) = el.as_token() else {
            return;
        };
        let text = token.text();
        if text.is_ascii() || text.starts_with('`') {
            return;
        }

        let mut spelling = String::with_capacity(text.len());
        let mut lookalikes = Vec::new();
        for ch in text.chars() {
            if ch.is_ascii() {
                spelling.push(ch);
            } else if let Some(ascii) = ascii_lookalike(ch) {
                spelling.push(ascii);
                if !lookalikes.contains(&ch) {
                    lookalikes.push(ch);
                }
            } else {
                // A partial transliteration would mislabel ordinary names in
                // another language, so require a wholly ASCII-like spelling.
                return;
            }
        }

        let points = lookalikes
            .iter()
            .map(|&ch| format!("U+{:04X}", ch as u32))
            .collect::<Vec<_>>()
            .join(", ");
        sink.push(Diagnostic {
            rule: self.id(),
            severity: Default::default(),
            path: Default::default(),
            range: token.text_range(),
            message: ViolationData::new(
                self.id(),
                format!("identifier `{text}` contains ASCII lookalikes ({points}); likely ASCII spelling is `{spelling}`"),
            )
            .with_suggestion("Check which spelling is intended; R treats these as different names."),
            fix: None,
        });
    }
}

fn ascii_lookalike(ch: char) -> Option<char> {
    // This deliberately narrow selection follows the letter shapes in Unicode
    // 16.0 confusables.txt, rather than normalizing or transliterating names.
    // https://www.unicode.org/Public/security/16.0.0/confusables.txt
    Some(match ch {
        'а' => 'a',
        'А' => 'A',
        'В' => 'B',
        'с' => 'c',
        'С' => 'C',
        'е' => 'e',
        'Е' => 'E',
        'Н' => 'H',
        'і' => 'i',
        'ј' => 'j',
        'Ј' => 'J',
        'К' => 'K',
        'М' => 'M',
        'о' => 'o',
        'О' => 'O',
        'р' => 'p',
        'Р' => 'P',
        'ѕ' => 's',
        'Ѕ' => 'S',
        'Т' => 'T',
        'х' => 'x',
        'Х' => 'X',
        'у' => 'y',
        'У' => 'Y',
        'Ａ'..='Ｚ' | 'ａ'..='ｚ' => char::from_u32(ch as u32 - 0xfee0)?,
        _ => return None,
    })
}
