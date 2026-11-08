//! RTF interpreter: tokens to blocks (rules R-11).

use std::collections::HashMap;

use encoding_rs::{Encoding, MACINTOSH, WINDOWS_1252};
use plaindoc_core::{
    Block, Cell, Level, ListItem, NoteKind, ParseError, Result, Warning, WarningKind,
};

mod blocks;
mod pict;

use super::MAX_GROUP_DEPTH;
use super::charset::{charset_codepage, codepage};
use super::lexer::{Lexer, Tok};
use crate::common::{CellBudget, cell_images_warning, finish_table};

/// Cell definitions (`\cellx`) kept per row; more are ignored.
const MAX_CELL_DEFS: usize = 16_384;

/// Destinations whose content is not document text.
const SKIPPED: &[&str] = &[
    "colortbl",
    "stylesheet",
    "nonshppict",
    "fldinst",
    "filetbl",
    "revtbl",
    "listtable",
    "listoverridetable",
    "xe",
    "tc",
    "txe",
    "rxe",
    "bkmkstart",
    "bkmkend",
    "private",
    "themedata",
    "colorschememapping",
    "datastore",
    "latentstyles",
    "generator",
    "userprops",
    "docvar",
    "pgdsctbl",
    "template",
    "mmathPr",
    "xmlnstbl",
    "rsidtbl",
    "wgrffmtfilter",
    "passwordhash",
    "objdata",
    "objclass",
    "objname",
    "picprop",
    "blipuid",
    "atnid",
    "atnauthor",
    "atndate",
    "atnref",
    "atnparent",
    "atntime",
    "pn",
    "pnseclvl",
    "listpicture",
    "background",
    "fchars",
    "lchars",
    "protusertbl",
    "nonesttables",
    "ftnsep",
    "ftnsepc",
    "ftncn",
    "aftnsep",
    "aftnsepc",
    "aftncn",
];

pub(crate) struct Parsed {
    pub blocks: Vec<Block>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub warnings: Vec<Warning>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Main,
    Note(usize),
    Title,
    Author,
    Label,
    Ignore,
    Info,
    FontTable,
    /// A `\pict` group whose data is taken (pictures on).
    Pict,
}

#[derive(Clone, Copy)]
struct Group {
    target: Target,
    uc: usize,
    font: Option<i32>,
    fresh: bool,
    star: bool,
    owns_note: bool,
    owns_pict: bool,
}

struct NoteBuf {
    kind: NoteKind,
    text: String,
}

#[derive(Clone, Copy, Default)]
struct CellDef {
    right: i32,
    hcont: bool,
    vcont: bool,
    vmerge: bool,
}

struct Interp {
    groups: Vec<Group>,
    cp: &'static Encoding,
    deff: Option<i32>,
    fonts: HashMap<i32, &'static Encoding>,
    buf: Vec<u8>,
    skip: usize,
    high: Option<u16>,
    upr: bool,
    run: String,
    label: String,
    title: String,
    author: String,
    notes: Vec<NoteBuf>,
    outline: Option<u8>,
    ls: Option<i32>,
    ilvl: usize,
    intbl: bool,
    defs: Vec<CellDef>,
    pending_def: CellDef,
    row_header: bool,
    row: Vec<Vec<Block>>,
    cell: Vec<Block>,
    table: Vec<Vec<Cell>>,
    header_rows: usize,
    vorigin: HashMap<i32, (usize, usize)>,
    lists: Vec<(bool, Vec<ListItem>)>,
    out: Vec<Block>,
    headers: Vec<Block>,
    footers: Vec<Block>,
    pending: Vec<Block>,
    /// Notes of list items: emitted right after the list closes.
    list_notes: Vec<Block>,
    /// Notes of table cells: emitted right after the table closes.
    table_notes: Vec<Block>,
    budget: CellBudget,
    error: Option<ParseError>,
    cell_images: usize,
    nested: bool,
    warnings: Vec<Warning>,
    /// Pictures on: the largest one taken; `None`: `\pict` gives a bare mark.
    pics: Option<u64>,
    pict: Option<pict::Pict>,
}

fn tidy(s: &str) -> String {
    s.trim().to_string()
}

fn ordered_label(label: &str) -> bool {
    let l = label.trim();
    l.chars().next().is_some_and(char::is_alphanumeric) && l.ends_with(['.', ')'])
}

pub(crate) fn parse(bytes: &[u8], pics: Option<u64>) -> Result<Parsed> {
    let head = bytes.trim_ascii_start();
    if !head.starts_with(b"{\\rtf") {
        return Err(ParseError::Corrupt("rtf: missing {\\rtf header".into()));
    }
    let mut it = Interp::new();
    it.pics = pics;
    for tok in Lexer::new(head) {
        it.step(tok);
        if let Some(e) = it.error.take() {
            return Err(e);
        }
        if it.groups.is_empty() {
            break;
        }
    }
    Ok(it.finish())
}

impl Interp {
    fn new() -> Self {
        Interp {
            groups: Vec::new(),
            cp: WINDOWS_1252,
            deff: None,
            fonts: HashMap::new(),
            buf: Vec::new(),
            skip: 0,
            high: None,
            upr: false,
            run: String::new(),
            label: String::new(),
            title: String::new(),
            author: String::new(),
            notes: Vec::new(),
            outline: None,
            ls: None,
            ilvl: 0,
            intbl: false,
            defs: Vec::new(),
            pending_def: CellDef::default(),
            row_header: false,
            row: Vec::new(),
            cell: Vec::new(),
            table: Vec::new(),
            header_rows: 0,
            vorigin: HashMap::new(),
            lists: Vec::new(),
            out: Vec::new(),
            headers: Vec::new(),
            footers: Vec::new(),
            pending: Vec::new(),
            list_notes: Vec::new(),
            table_notes: Vec::new(),
            budget: CellBudget::new(),
            error: None,
            cell_images: 0,
            nested: false,
            warnings: Vec::new(),
            pics: None,
            pict: None,
        }
    }

    fn top(&self) -> &Group {
        self.groups.last().expect("group stack is not empty")
    }

    fn top_mut(&mut self) -> &mut Group {
        self.groups.last_mut().expect("group stack is not empty")
    }

    fn takes_text(&self) -> bool {
        matches!(
            self.top().target,
            Target::Main | Target::Note(_) | Target::Title | Target::Author | Target::Label
        )
    }

    fn step(&mut self, tok: Tok) {
        match tok {
            Tok::Open => {
                self.flush();
                self.open();
            }
            Tok::Close => self.close(),
            Tok::Text(mut t) => {
                self.top_mut().fresh = false;
                let n = self.skip.min(t.len());
                t = &t[n..];
                self.skip -= n;
                if !t.is_empty() && self.takes_text() {
                    self.buf.extend_from_slice(t);
                } else if self.top().target == Target::Pict
                    && let Some(p) = &mut self.pict
                {
                    p.text(t);
                }
            }
            Tok::Hex(b) => {
                self.top_mut().fresh = false;
                if self.skip > 0 {
                    self.skip -= 1;
                } else if self.takes_text() {
                    self.buf.push(b);
                }
            }
            Tok::Word(n, p) => {
                self.flush();
                self.word(n, p);
            }
            Tok::Symbol(b'*') => {
                self.flush();
                let g = self.top_mut();
                if g.fresh {
                    g.star = true;
                }
            }
            Tok::Symbol(c) => {
                self.flush();
                self.top_mut().fresh = false;
                if self.skip > 0 {
                    self.skip -= 1;
                } else {
                    self.symbol(c);
                }
            }
        }
    }

    fn open(&mut self) {
        if self.groups.len() >= MAX_GROUP_DEPTH {
            self.error = Some(ParseError::Limit {
                kind: "rtf group depth".into(),
                limit: MAX_GROUP_DEPTH as u64,
            });
            return;
        }
        let mut g = self.groups.last().copied().unwrap_or(Group {
            target: Target::Main,
            uc: 1,
            font: None,
            fresh: false,
            star: false,
            owns_note: false,
            owns_pict: false,
        });
        g.fresh = true;
        g.star = false;
        g.owns_note = false;
        g.owns_pict = false;
        if std::mem::take(&mut self.upr) {
            g.target = Target::Ignore;
        }
        self.groups.push(g);
        self.skip = 0;
    }

    fn close(&mut self) {
        self.flush();
        let Some(g) = self.groups.pop() else { return };
        if g.owns_pict
            && let Some(p) = self.pict.take()
        {
            self.pending.push(p.finish());
        }
        // notes nest with their groups, so the owner's note is always the last one
        if g.owns_note
            && let Some(note) = self.notes.pop()
        {
            let text = tidy(&note.text);
            if !text.is_empty() {
                let kind = note.kind;
                let block = Block::Note {
                    kind,
                    text,
                    bbox: None,
                };
                match kind {
                    NoteKind::Header => self.headers.push(block),
                    NoteKind::Footer => self.footers.push(block),
                    _ => self.pending.push(block),
                }
            }
        }
        self.upr = false;
        self.skip = 0;
    }

    fn encoding(&self) -> &'static Encoding {
        self.top()
            .font
            .or(self.deff)
            .and_then(|f| self.fonts.get(&f).copied())
            .unwrap_or(self.cp)
    }

    fn flush(&mut self) {
        if self.buf.is_empty() || self.groups.is_empty() {
            return;
        }
        let enc = self.encoding();
        let buf = std::mem::take(&mut self.buf);
        let (s, _) = enc.decode_without_bom_handling(&buf);
        self.push_str(&s);
        self.buf = buf;
        self.buf.clear();
    }

    fn push_str(&mut self, s: &str) {
        match self.top().target {
            Target::Main => self.run.push_str(s),
            Target::Note(i) => self.notes[i].text.push_str(s),
            Target::Title => self.title.push_str(s),
            Target::Author => self.author.push_str(s),
            Target::Label => self.label.push_str(s),
            _ => {}
        }
    }

    fn symbol(&mut self, c: u8) {
        match c {
            b'{' | b'}' | b'\\' => self.push_str(char::from(c).encode_utf8(&mut [0; 4])),
            b'~' => self.push_str(" "),
            b'_' => self.push_str("-"),
            b'\n' => self.end_para(),
            _ => {}
        }
    }

    fn word(&mut self, name: &str, p: Option<i32>) {
        let fresh = std::mem::replace(&mut self.top_mut().fresh, false);
        let g = *self.top();
        if g.target == Target::Ignore {
            return;
        }
        if self.skip > 0 && name != "u" {
            self.skip -= 1;
            return;
        }
        if fresh && self.destination(name, g) {
            return;
        }
        match g.target {
            Target::FontTable => {
                self.font_table(name, p);
            }
            Target::Pict => {
                if let Some(pc) = &mut self.pict {
                    pc.word(name);
                }
            }
            Target::Info => {}
            t => self.control(name, p, t),
        }
    }

    fn font_table(&mut self, name: &str, p: Option<i32>) {
        let Some(n) = p else { return };
        if name == "f" {
            self.top_mut().font = Some(n);
            return;
        }
        let Some(f) = self.top().font else { return };
        let cp = match name {
            "fcharset" => charset_codepage(n),
            "cpg" => Some(n),
            _ => return,
        };
        if let Some(e) = cp.and_then(codepage) {
            self.fonts.insert(f, e);
        }
    }

    fn new_note(&mut self, kind: NoteKind) -> Target {
        self.notes.push(NoteBuf {
            kind,
            text: String::new(),
        });
        self.top_mut().owns_note = true;
        Target::Note(self.notes.len() - 1)
    }

    fn destination(&mut self, name: &str, g: Group) -> bool {
        let cur = g.target;
        let target = match name {
            "fonttbl" => Target::FontTable,
            "info" => Target::Info,
            "title" if cur == Target::Info => Target::Title,
            "author" if cur == Target::Info => Target::Author,
            "listtext" | "pntext" => Target::Label,
            "footnote" => self.new_note(NoteKind::Footnote),
            "annotation" => self.new_note(NoteKind::Comment),
            "header" | "headerl" | "headerr" | "headerf" => self.new_note(NoteKind::Header),
            "footer" | "footerl" | "footerr" | "footerf" => self.new_note(NoteKind::Footer),
            "pict" => {
                if cur == Target::Main {
                    if self.intbl {
                        self.cell_images += 1;
                    } else if let Some(max) = self.pics {
                        self.pict = Some(pict::Pict::new(max));
                        self.top_mut().owns_pict = true;
                        self.top_mut().target = Target::Pict;
                        return true;
                    } else {
                        self.pending.push(Block::Image {
                            alt: None,
                            ocr_text: None,
                            bbox: None,
                            image: None,
                        });
                    }
                }
                Target::Ignore
            }
            "field" | "fldrslt" | "ud" | "shppict" => cur,
            "upr" => {
                self.upr = true;
                cur
            }
            n if SKIPPED.contains(&n) => Target::Ignore,
            _ if g.star => Target::Ignore,
            _ => return false,
        };
        self.top_mut().target = target;
        true
    }

    fn control(&mut self, name: &str, p: Option<i32>, target: Target) {
        let main = target == Target::Main;
        match name {
            "emdash" => self.push_str("\u{2014}"),
            "endash" => self.push_str("\u{2013}"),
            "emspace" => self.push_str("\u{2003}"),
            "enspace" => self.push_str("\u{2002}"),
            "bullet" => self.push_str("\u{2022}"),
            "lquote" => self.push_str("\u{2018}"),
            "rquote" => self.push_str("\u{2019}"),
            "ldblquote" => self.push_str("\u{201C}"),
            "rdblquote" => self.push_str("\u{201D}"),
            "line" => self.push_str("\n"),
            "tab" => self.push_str("\t"),
            "par" | "sect" | "page" => self.end_para(),
            "u" => {
                if let Some(v) = p {
                    self.unicode(v);
                }
            }
            "uc" => self.top_mut().uc = p.unwrap_or(1).clamp(0, 64) as usize,
            "f" => self.top_mut().font = p,
            "deff" => self.deff = p,
            "ansicpg" => {
                if let Some(e) = p.and_then(codepage) {
                    self.cp = e;
                }
            }
            "mac" => self.cp = MACINTOSH,
            "cell" | "row" | "nestcell" | "nestrow" if !main => self.end_para(),
            _ if !main => {}
            "pard" => {
                self.outline = None;
                self.ls = None;
                self.ilvl = 0;
                self.intbl = false;
            }
            "outlinelevel" => self.outline = p.map(|n| n.clamp(0, 5) as u8),
            "ls" => self.ls = p,
            "ilvl" => self.ilvl = p.unwrap_or(0).clamp(0, 8) as usize,
            "intbl" => self.intbl = true,
            "itap" => {
                if p.is_some_and(|n| n > 1) {
                    self.nested = true;
                }
            }
            "trowd" => {
                self.defs.clear();
                self.pending_def = CellDef::default();
                self.row_header = false;
            }
            "trhdr" => self.row_header = true,
            "clmrg" => self.pending_def.hcont = true,
            "clvmgf" => self.pending_def.vmerge = true,
            "clvmrg" => self.pending_def.vcont = true,
            "cellx" => {
                let mut d = std::mem::take(&mut self.pending_def);
                d.right = p.unwrap_or(0);
                if self.defs.len() < MAX_CELL_DEFS {
                    self.defs.push(d);
                }
            }
            "cell" => self.end_cell(),
            "row" => self.end_row(),
            "nestcell" | "nestrow" => {
                self.nested = true;
                self.end_main_para();
            }
            _ => {}
        }
    }

    fn unicode(&mut self, v: i32) {
        let v = if v < 0 { v + 65536 } else { v };
        let Ok(u) = u16::try_from(v) else { return };
        let c = match (self.high.take(), u) {
            (_, 0xD800..=0xDBFF) => {
                self.high = Some(u);
                None
            }
            (Some(h), 0xDC00..=0xDFFF) => {
                char::from_u32(0x10000 + ((u32::from(h) - 0xD800) << 10) + (u32::from(u) - 0xDC00))
            }
            (_, u) => char::from_u32(u32::from(u)),
        };
        if let Some(c) = c {
            self.push_str(c.encode_utf8(&mut [0; 4]));
        }
        self.skip = self.top().uc;
    }
}
