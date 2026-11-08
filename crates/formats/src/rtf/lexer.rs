//! RTF tokenizer: groups, control words/symbols, hex escapes and raw text runs.

/// One RTF token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tok<'a> {
    Open,
    Close,
    /// Control word with its optional numeric parameter.
    Word(&'a str, Option<i32>),
    /// Control symbol (`\~`, `\-`, `\*`, ...; `\` + line break gives `\n`).
    Symbol(u8),
    /// `\'hh` escape.
    Hex(u8),
    /// Raw text bytes (line breaks excluded).
    Text(&'a [u8]),
}

const MAX_WORD: usize = 32;
const MAX_DIGITS: usize = 11;

pub(crate) struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub(crate) fn new(src: &'a [u8]) -> Self {
        Lexer { src, pos: 0 }
    }

    fn peek(&self, off: usize) -> Option<u8> {
        self.src.get(self.pos + off).copied()
    }

    fn control(&mut self) -> Option<Tok<'a>> {
        // `self.pos` is just after the backslash.
        let b = self.peek(0)?;
        if b.is_ascii_alphabetic() {
            let start = self.pos;
            while self.peek(0).is_some_and(|c| c.is_ascii_alphabetic()) {
                self.pos += 1;
            }
            let end = start + (self.pos - start).min(MAX_WORD);
            let name = std::str::from_utf8(&self.src[start..end]).unwrap_or("");
            let neg =
                self.peek(0) == Some(b'-') && self.peek(1).is_some_and(|c| c.is_ascii_digit());
            let mut param = None;
            if neg || self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                if neg {
                    self.pos += 1;
                }
                let mut n: i64 = 0;
                let mut digits = 0;
                while let Some(c) = self.peek(0).filter(u8::is_ascii_digit) {
                    if digits < MAX_DIGITS {
                        n = n * 10 + i64::from(c - b'0');
                        digits += 1;
                    }
                    self.pos += 1;
                }
                let n = if neg { -n } else { n };
                param = Some(n.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32);
            }
            if self.peek(0) == Some(b' ') {
                self.pos += 1;
            }
            if name == "bin"
                && let Some(n) = param.filter(|n| *n > 0)
            {
                self.pos = self.pos.saturating_add(n as usize).min(self.src.len());
            }
            return Some(Tok::Word(name, param));
        }
        self.pos += 1;
        match b {
            b'\'' => {
                let hex = |c: Option<u8>| c.and_then(|c| (c as char).to_digit(16));
                if let (Some(h), Some(l)) = (hex(self.peek(0)), hex(self.peek(1))) {
                    self.pos += 2;
                    Some(Tok::Hex((h * 16 + l) as u8))
                } else {
                    Some(Tok::Symbol(b'\''))
                }
            }
            b'\r' => {
                if self.peek(0) == Some(b'\n') {
                    self.pos += 1;
                }
                Some(Tok::Symbol(b'\n'))
            }
            _ => Some(Tok::Symbol(b)),
        }
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Tok<'a>;

    fn next(&mut self) -> Option<Tok<'a>> {
        loop {
            let b = self.peek(0)?;
            match b {
                b'{' => {
                    self.pos += 1;
                    return Some(Tok::Open);
                }
                b'}' => {
                    self.pos += 1;
                    return Some(Tok::Close);
                }
                b'\r' | b'\n' => self.pos += 1,
                b'\\' => {
                    self.pos += 1;
                    return self.control();
                }
                _ => {
                    let start = self.pos;
                    while self
                        .peek(0)
                        .is_some_and(|c| !matches!(c, b'{' | b'}' | b'\\' | b'\r' | b'\n'))
                    {
                        self.pos += 1;
                    }
                    return Some(Tok::Text(&self.src[start..self.pos]));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Tok::*;

    #[test]
    fn tokens() {
        let t: Vec<_> =
            Lexer::new(b"{\\rtf1\\ansi\\ansicpg1251 Hi\\'e0\\u-3913?\\par\r\n\\~x\\-\\bin3 {}}y}")
                .collect();
        assert_eq!(
            t,
            vec![
                Open,
                Word("rtf", Some(1)),
                Word("ansi", None),
                Word("ansicpg", Some(1251)),
                Text(b"Hi"),
                Hex(0xe0),
                Word("u", Some(-3913)),
                Text(b"?"),
                Word("par", None),
                Symbol(b'~'),
                Text(b"x"),
                Symbol(b'-'),
                Word("bin", Some(3)),
                Text(b"y"),
                Close,
            ]
        );
    }

    #[test]
    fn negative_bin_skips_nothing() {
        let t: Vec<_> = Lexer::new(b"\\bin-3 ab").collect();
        assert_eq!(t, vec![Word("bin", Some(-3)), Text(b"ab")]);
    }
}
