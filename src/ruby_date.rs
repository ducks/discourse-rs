//! `Date._parse` (ext/date/date_parse.c of the date gem 3.4.1, as built
//! without TIGHT_PARSER): the fields a free-form date string names, each
//! sub-parser blanking out what it matched before the next one runs.
//! Only ASCII strings are read (callers refuse others), so Onigmo's
//! ASCII `\b`, `\s` and POSIX classes are the regex crate's.

use std::sync::LazyLock;

use fancy_regex::Regex;

/// The hash `Date._parse` returns (the keys Time.parse reads, and the
/// ones the sub-parsers consult).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DateHash {
    pub year: Option<i64>,
    pub mon: Option<i64>,
    pub mday: Option<i64>,
    pub yday: Option<i64>,
    pub hour: Option<i64>,
    pub min: Option<i64>,
    pub sec: Option<i64>,
    pub sec_fraction: bool,
    pub zone: Option<String>,
    pub wday: Option<i64>,
    pub cwyear: Option<i64>,
    pub cweek: Option<i64>,
    pub cwday: Option<i64>,
    comp: bool,
    bc: bool,
}

const ABBR_DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const ABBR_MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const ABBR_DAYS_RE: &str = "sun|mon|tue|wed|thu|fri|sat";
const ABBR_MONTHS_RE: &str = "jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec";
const NUMBER: &str = r"(?<!\d)\d";

fn re(source: String) -> Regex {
    Regex::new(&source).expect("date_parse patterns compile")
}

/// `rb_str_to_inum(s, 10, 0)`: leading whitespace, a sign, digits (with
/// single underscores), stopping at anything else; 0 for none.
fn str2num(s: &str) -> i64 {
    crate::ruby::to_i(s)
}

fn day_num(s: &str) -> i64 {
    let lower = s.to_ascii_lowercase();
    ABBR_DAYS
        .iter()
        .position(|d| lower.starts_with(d))
        .unwrap_or(ABBR_DAYS.len()) as i64
}

fn mon_num(s: &str) -> i64 {
    let lower = s.to_ascii_lowercase();
    ABBR_MONTHS
        .iter()
        .position(|m| lower.starts_with(m))
        .unwrap_or(ABBR_MONTHS.len()) as i64
        + 1
}

type Caps = Vec<Option<String>>;

/// `subx`: the first match blanked to one space, then its callback.
fn subx(
    s: &mut String,
    pattern: &Regex,
    h: &mut DateHash,
    cb: impl FnOnce(&Caps, &mut DateHash),
) -> bool {
    let Ok(Some(caps)) = pattern.captures(s) else {
        return false;
    };
    let whole = caps.get(0).unwrap();
    let (begin, end) = (whole.start(), whole.end());
    let owned: Caps = (0..caps.len())
        .map(|i| caps.get(i).map(|m| m.as_str().to_string()))
        .collect();
    s.replace_range(begin..end, " ");
    cb(&owned, h);
    true
}

fn cap(c: &Caps, i: usize) -> Option<&str> {
    c.get(i).and_then(|m| m.as_deref())
}

fn is_sign(b: u8) -> bool {
    b == b'-' || b == b'+'
}

fn digit_span(s: &[u8]) -> usize {
    s.iter().take_while(|b| b.is_ascii_digit()).count()
}

/// `s3e(hash, y, m, d, bc)`: a year, month and day in whatever order the
/// sub-parser found them, sorted out by their shapes.
fn s3e(h: &mut DateHash, y: Option<String>, m: Option<String>, d: Option<String>, bc: bool) {
    let (mut y, mut m, mut d) = (y, m, d);
    let mut c: Option<bool> = None;
    if y.is_some() && m.is_some() && d.is_none() {
        let (oy, om, od) = (y, m, d);
        y = od;
        m = oy;
        d = om;
    }
    if y.is_none() {
        if d.as_ref().is_some_and(|d| d.len() > 2) {
            y = d.take();
        }
        if d.as_ref().is_some_and(|d| d.starts_with('\'')) {
            y = d.take();
        }
    }
    if let Some(ys) = y.clone() {
        let b = ys.as_bytes();
        let mut i = 0;
        while i < b.len() && !is_sign(b[i]) && !b[i].is_ascii_digit() {
            i += 1;
        }
        if i < b.len() {
            let bp = i;
            if is_sign(b[i]) {
                i += 1;
            }
            let l = digit_span(&b[i..]);
            let ep = i + l;
            // `if (*ep)`: something follows the digits.
            if ep < b.len() {
                y = d.take();
                d = Some(ys[bp..ep].to_string());
            }
        }
    }
    if let Some(ms) = &m
        && (ms.starts_with('\'') || ms.len() > 2)
    {
        // us -> be
        let (oy, om, od) = (y, m, d);
        y = om;
        m = od;
        d = oy;
    }
    if let Some(ds) = &d
        && (ds.starts_with('\'') || ds.len() > 2)
    {
        std::mem::swap(&mut y, &mut d);
    }
    if let Some(ys) = &y {
        let b = ys.as_bytes();
        let mut i = 0;
        while i < b.len() && !is_sign(b[i]) && !b[i].is_ascii_digit() {
            i += 1;
        }
        if i < b.len() {
            let bp = i;
            let mut sign = false;
            if is_sign(b[i]) {
                i += 1;
                sign = true;
            }
            if sign {
                c = Some(false);
            }
            let l = digit_span(&b[i..]);
            if l > 2 {
                c = Some(false);
            }
            h.year = Some(str2num(&ys[bp..i + l]));
        }
    }
    if bc {
        h.bc = true;
    }
    if let Some(ms) = &m {
        let b = ms.as_bytes();
        if let Some(start) = b.iter().position(u8::is_ascii_digit) {
            let l = digit_span(&b[start..]);
            h.mon = Some(str2num(&ms[start..start + l]));
        }
    }
    if let Some(ds) = &d {
        let b = ds.as_bytes();
        if let Some(start) = b.iter().position(u8::is_ascii_digit) {
            let l = digit_span(&b[start..]);
            h.mday = Some(str2num(&ds[start..start + l]));
        }
    }
    if let Some(c) = c {
        h.comp = c;
    }
}

fn parse_day(s: &mut String, h: &mut DateHash) -> bool {
    static PAT: LazyLock<Regex> =
        LazyLock::new(|| re(format!(r"(?i)\b({ABBR_DAYS_RE})[^-/\d\s]*")));
    subx(s, &PAT, h, |c, h| {
        h.wday = Some(day_num(cap(c, 1).unwrap()))
    })
}

fn parse_time(s: &mut String, h: &mut DateHash) -> bool {
    static PAT: LazyLock<Regex> = LazyLock::new(|| {
        re(format!(
            r"(?i)({NUMBER}+\s*(?:(?::\s*\d+(?:\s*:\s*\d+(?:[,.]\d*)?)?|h(?:\s*\d+m?(?:\s*\d+s?)?)?)(?:\s*[ap](?:m\b|\.m\.))?|[ap](?:m\b|\.m\.)))(?:\s*((?:gmt|utc?)?[-+]\d+(?:[,.:]\d+(?::\d+)?)?|(?-i:[[:alpha:].\s]+)(?:standard|daylight)\stime\b|(?-i:[[:alpha:]]+)(?:\sdst)?\b))?"
        ))
    });
    static TIME: LazyLock<Regex> = LazyLock::new(|| {
        re(
            r"(?i)\A(\d+)h?(?:\s*:?\s*(\d+)m?(?:\s*:?\s*(\d+)(?:[,.](\d+))?s?)?)?(?:\s*([ap])(?:m\b|\.m\.))?"
                .to_string(),
        )
    });
    subx(s, &PAT, h, |c, h| {
        if let Some(zone) = cap(c, 2) {
            h.zone = Some(zone.to_string());
        }
        let s1 = cap(c, 1).unwrap_or("");
        let Ok(Some(m)) = TIME.captures(s1) else {
            return;
        };
        let mut hour = str2num(m.get(1).unwrap().as_str());
        if let Some(p) = m.get(5) {
            hour %= 12;
            if p.as_str().eq_ignore_ascii_case("p") {
                hour += 12;
            }
        }
        h.hour = Some(hour);
        if let Some(min) = m.get(2) {
            h.min = Some(str2num(min.as_str()));
        }
        if let Some(sec) = m.get(3) {
            h.sec = Some(str2num(sec.as_str()));
        }
        if m.get(4).is_some() {
            h.sec_fraction = true;
        }
    })
}

const ERA: &str = r"(c(?:e|\.e\.)|b(?:ce|\.c\.e\.)|a(?:d|\.d\.)|b(?:c|\.c\.))";

fn is_bc(b: Option<&str>) -> bool {
    b.is_some_and(|b| b.starts_with(['B', 'b']))
}

fn parse_eu(s: &mut String, h: &mut DateHash) -> bool {
    static PAT: LazyLock<Regex> = LazyLock::new(|| {
        re(format!(
            r"(?i)('?{NUMBER}+)[^-\d\s]*\s*({ABBR_MONTHS_RE})[^-\d\s']*(?:\s*(?:\b{ERA}(?!(?<!\.)[a-z]))?\s*('?-?\d+(?:(?:st|nd|rd|th)\b)?))?"
        ))
    });
    subx(s, &PAT, h, |c, h| {
        let mon = mon_num(cap(c, 2).unwrap()).to_string();
        s3e(
            h,
            cap(c, 4).map(str::to_string),
            Some(mon),
            cap(c, 1).map(str::to_string),
            is_bc(cap(c, 3)),
        );
    })
}

fn parse_us(s: &mut String, h: &mut DateHash) -> bool {
    static PAT: LazyLock<Regex> = LazyLock::new(|| {
        re(format!(
            r"(?i)\b({ABBR_MONTHS_RE})[^-\d\s']*\s*('?\d+)[^-\d\s']*(?:\s*+,?\s*+{ERA}?\s*('?-?\d+))?"
        ))
    });
    subx(s, &PAT, h, |c, h| {
        let mon = mon_num(cap(c, 1).unwrap()).to_string();
        s3e(
            h,
            cap(c, 4).map(str::to_string),
            Some(mon),
            cap(c, 2).map(str::to_string),
            is_bc(cap(c, 3)),
        );
    })
}

fn parse_iso(s: &mut String, h: &mut DateHash) -> bool {
    static PAT: LazyLock<Regex> =
        LazyLock::new(|| re(format!(r"('?[-+]?{NUMBER}+)-(\d+)-('?-?\d+)")));
    subx(s, &PAT, h, |c, h| {
        s3e(
            h,
            cap(c, 1).map(str::to_string),
            cap(c, 2).map(str::to_string),
            cap(c, 3).map(str::to_string),
            false,
        );
    })
}

fn parse_iso2(s: &mut String, h: &mut DateHash) -> bool {
    static ISO21: LazyLock<Regex> =
        LazyLock::new(|| re(r"(?i)\b(\d{2}|\d{4})?-?w(\d{2})(?:-?(\d))?\b".to_string()));
    static ISO22: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)-w-(\d)\b".to_string()));
    static ISO23: LazyLock<Regex> = LazyLock::new(|| re(r"--(\d{2})?-(\d{2})\b".to_string()));
    static ISO24: LazyLock<Regex> = LazyLock::new(|| re(r"--(\d{2})(\d{2})?\b".to_string()));
    static ISO25_0: LazyLock<Regex> =
        LazyLock::new(|| re(r"[,.](\d{2}|\d{4})-\d{3}\b".to_string()));
    static ISO25: LazyLock<Regex> = LazyLock::new(|| re(r"\b(\d{2}|\d{4})-(\d{3})\b".to_string()));
    static ISO26_0: LazyLock<Regex> = LazyLock::new(|| re(r"\d-\d{3}\b".to_string()));
    static ISO26: LazyLock<Regex> = LazyLock::new(|| re(r"\b-(\d{3})\b".to_string()));
    if subx(s, &ISO21, h, |c, h| {
        if let Some(y) = cap(c, 1) {
            h.cwyear = Some(str2num(y));
        }
        h.cweek = Some(str2num(cap(c, 2).unwrap()));
        if let Some(d) = cap(c, 3) {
            h.cwday = Some(str2num(d));
        }
    }) {
        return true;
    }
    if subx(s, &ISO22, h, |c, h| {
        h.cwday = Some(str2num(cap(c, 1).unwrap()))
    }) {
        return true;
    }
    if subx(s, &ISO23, h, |c, h| {
        if let Some(m) = cap(c, 1) {
            h.mon = Some(str2num(m));
        }
        h.mday = Some(str2num(cap(c, 2).unwrap()));
    }) {
        return true;
    }
    if subx(s, &ISO24, h, |c, h| {
        h.mon = Some(str2num(cap(c, 1).unwrap()));
        if let Some(d) = cap(c, 2) {
            h.mday = Some(str2num(d));
        }
    }) {
        return true;
    }
    if !matches!(ISO25_0.is_match(s), Ok(true))
        && subx(s, &ISO25, h, |c, h| {
            h.year = Some(str2num(cap(c, 1).unwrap()));
            h.yday = Some(str2num(cap(c, 2).unwrap()));
        })
    {
        return true;
    }
    if !matches!(ISO26_0.is_match(s), Ok(true))
        && subx(s, &ISO26, h, |c, h| {
            h.yday = Some(str2num(cap(c, 1).unwrap()))
        })
    {
        return true;
    }
    false
}

fn gengo(c: char) -> i64 {
    match c.to_ascii_lowercase() {
        'm' => 1867,
        't' => 1911,
        's' => 1925,
        'h' => 1988,
        'r' => 2018,
        _ => 0,
    }
}

fn parse_jis(s: &mut String, h: &mut DateHash) -> bool {
    static PAT: LazyLock<Regex> =
        LazyLock::new(|| re(r"(?i)\b([mtshr])(\d+)\.(\d+)\.(\d+)".to_string()));
    subx(s, &PAT, h, |c, h| {
        let era = gengo(cap(c, 1).unwrap().chars().next().unwrap());
        h.year = Some(str2num(cap(c, 2).unwrap()) + era);
        h.mon = Some(str2num(cap(c, 3).unwrap()));
        h.mday = Some(str2num(cap(c, 4).unwrap()));
    })
}

fn parse_vms(s: &mut String, h: &mut DateHash) -> bool {
    static VMS11: LazyLock<Regex> = LazyLock::new(|| {
        re(format!(
            r"(?i)('?-?{NUMBER}+)-({ABBR_MONTHS_RE})[^-/.]*-('?-?\d+)"
        ))
    });
    static VMS12: LazyLock<Regex> = LazyLock::new(|| {
        re(format!(
            r"(?i)\b({ABBR_MONTHS_RE})[^-/.]*-('?-?\d+)(?:-('?-?\d+))?"
        ))
    });
    if subx(s, &VMS11, h, |c, h| {
        let mon = mon_num(cap(c, 2).unwrap()).to_string();
        s3e(
            h,
            cap(c, 3).map(str::to_string),
            Some(mon),
            cap(c, 1).map(str::to_string),
            false,
        );
    }) {
        return true;
    }
    subx(s, &VMS12, h, |c, h| {
        let mon = mon_num(cap(c, 1).unwrap()).to_string();
        s3e(
            h,
            cap(c, 3).map(str::to_string),
            Some(mon),
            cap(c, 2).map(str::to_string),
            false,
        );
    })
}

fn parse_sla(s: &mut String, h: &mut DateHash) -> bool {
    static PAT: LazyLock<Regex> = LazyLock::new(|| {
        re(format!(
            r"(?i)('?-?{NUMBER}+)/\s*('?\d+)(?:\D\s*('?-?\d+))?"
        ))
    });
    subx(s, &PAT, h, |c, h| {
        s3e(
            h,
            cap(c, 1).map(str::to_string),
            cap(c, 2).map(str::to_string),
            cap(c, 3).map(str::to_string),
            false,
        );
    })
}

fn parse_dot(s: &mut String, h: &mut DateHash) -> bool {
    static PAT: LazyLock<Regex> =
        LazyLock::new(|| re(format!(r"(?i)('?-?{NUMBER}+)\.\s*('?\d+)\.\s*('?-?\d+)")));
    subx(s, &PAT, h, |c, h| {
        s3e(
            h,
            cap(c, 1).map(str::to_string),
            cap(c, 2).map(str::to_string),
            cap(c, 3).map(str::to_string),
            false,
        );
    })
}

fn parse_year(s: &mut String, h: &mut DateHash) -> bool {
    static PAT: LazyLock<Regex> = LazyLock::new(|| re(r"'(\d+)\b".to_string()));
    subx(s, &PAT, h, |c, h| {
        h.year = Some(str2num(cap(c, 1).unwrap()))
    })
}

fn parse_mon(s: &mut String, h: &mut DateHash) -> bool {
    static PAT: LazyLock<Regex> = LazyLock::new(|| re(format!(r"(?i)\b({ABBR_MONTHS_RE})\S*")));
    subx(s, &PAT, h, |c, h| h.mon = Some(mon_num(cap(c, 1).unwrap())))
}

fn parse_mday(s: &mut String, h: &mut DateHash) -> bool {
    static PAT: LazyLock<Regex> = LazyLock::new(|| re(format!(r"(?i)({NUMBER}+)(st|nd|rd|th)\b")));
    subx(s, &PAT, h, |c, h| {
        h.mday = Some(str2num(cap(c, 1).unwrap()))
    })
}

/// `n2i(s, f, w)`
fn n2i(s: &str, f: usize, w: usize) -> i64 {
    s.as_bytes()[f..f + w]
        .iter()
        .fold(0, |v, b| v * 10 + i64::from(b - b'0'))
}

fn parse_ddd(s: &mut String, h: &mut DateHash) -> bool {
    static PAT: LazyLock<Regex> = LazyLock::new(|| {
        re(format!(
            r"(?i)([-+]?)({NUMBER}{{2,14}})(?:\s*t?\s*(\d{{2,6}})?(?:[,.](\d*))?)?(?:\s*(z\b|[-+]\d{{1,4}}\b|\[[-+]?\d[^\]]*\]))?"
        ))
    });
    subx(s, &PAT, h, |c, h| {
        let s1 = cap(c, 1);
        let s2 = cap(c, 2).unwrap();
        let s3 = cap(c, 3);
        let s4 = cap(c, 4);
        let s5 = cap(c, 5);
        let neg = s1.is_some_and(|s| s.starts_with('-'));
        let signed = |y: i64| if neg { -y } else { y };
        let l2 = s2.len();
        let time_only = s3.is_none() && s4.is_some();
        match l2 {
            2 => {
                if time_only {
                    h.sec = Some(n2i(s2, l2 - 2, 2));
                } else {
                    h.mday = Some(n2i(s2, 0, 2));
                }
            }
            4 => {
                if time_only {
                    h.sec = Some(n2i(s2, l2 - 2, 2));
                    h.min = Some(n2i(s2, l2 - 4, 2));
                } else {
                    h.mon = Some(n2i(s2, 0, 2));
                    h.mday = Some(n2i(s2, 2, 2));
                }
            }
            6 => {
                if time_only {
                    h.sec = Some(n2i(s2, l2 - 2, 2));
                    h.min = Some(n2i(s2, l2 - 4, 2));
                    h.hour = Some(n2i(s2, l2 - 6, 2));
                } else {
                    h.year = Some(signed(n2i(s2, 0, 2)));
                    h.mon = Some(n2i(s2, 2, 2));
                    h.mday = Some(n2i(s2, 4, 2));
                }
            }
            8 | 10 | 12 | 14 => {
                if time_only {
                    h.sec = Some(n2i(s2, l2 - 2, 2));
                    h.min = Some(n2i(s2, l2 - 4, 2));
                    h.hour = Some(n2i(s2, l2 - 6, 2));
                    h.mday = Some(n2i(s2, l2 - 8, 2));
                    if l2 >= 10 {
                        h.mon = Some(n2i(s2, l2 - 10, 2));
                    }
                    if l2 == 12 {
                        h.year = Some(signed(n2i(s2, l2 - 12, 2)));
                    }
                    if l2 == 14 {
                        h.year = Some(signed(n2i(s2, l2 - 14, 4)));
                        h.comp = false;
                    }
                } else {
                    h.year = Some(signed(n2i(s2, 0, 4)));
                    h.mon = Some(n2i(s2, 4, 2));
                    h.mday = Some(n2i(s2, 6, 2));
                    if l2 >= 10 {
                        h.hour = Some(n2i(s2, 8, 2));
                    }
                    if l2 >= 12 {
                        h.min = Some(n2i(s2, 10, 2));
                    }
                    if l2 >= 14 {
                        h.sec = Some(n2i(s2, 12, 2));
                    }
                    h.comp = false;
                }
            }
            3 => {
                if time_only {
                    h.sec = Some(n2i(s2, l2 - 2, 2));
                    h.min = Some(n2i(s2, l2 - 3, 1));
                } else {
                    h.yday = Some(n2i(s2, 0, 3));
                }
            }
            5 => {
                if time_only {
                    h.sec = Some(n2i(s2, l2 - 2, 2));
                    h.min = Some(n2i(s2, l2 - 4, 2));
                    h.hour = Some(n2i(s2, l2 - 5, 1));
                } else {
                    h.year = Some(signed(n2i(s2, 0, 2)));
                    h.yday = Some(n2i(s2, 2, 3));
                }
            }
            7 => {
                if time_only {
                    h.sec = Some(n2i(s2, l2 - 2, 2));
                    h.min = Some(n2i(s2, l2 - 4, 2));
                    h.hour = Some(n2i(s2, l2 - 6, 2));
                    h.mday = Some(n2i(s2, l2 - 7, 1));
                } else {
                    h.year = Some(signed(n2i(s2, 0, 4)));
                    h.yday = Some(n2i(s2, 4, 3));
                }
            }
            _ => {}
        }
        if let Some(s3) = s3 {
            let l3 = s3.len();
            if matches!(l3, 2 | 4 | 6) {
                if s4.is_some() {
                    h.sec = Some(n2i(s3, l3 - 2, 2));
                    if l3 >= 4 {
                        h.min = Some(n2i(s3, l3 - 4, 2));
                    }
                    if l3 >= 6 {
                        h.hour = Some(n2i(s3, l3 - 6, 2));
                    }
                } else {
                    h.hour = Some(n2i(s3, 0, 2));
                    if l3 >= 4 {
                        h.min = Some(n2i(s3, 2, 2));
                    }
                    if l3 >= 6 {
                        h.sec = Some(n2i(s3, 4, 2));
                    }
                }
            }
        }
        if s4.is_some() {
            h.sec_fraction = true;
        }
        if let Some(s5) = s5 {
            h.zone = Some(match s5.strip_prefix('[') {
                Some(inner) => {
                    let inner = inner.strip_suffix(']').unwrap_or(inner);
                    match inner.split_once(':') {
                        Some((_, zone)) => zone.to_string(),
                        None => inner.to_string(),
                    }
                }
                None => s5.to_string(),
            });
        }
    })
}

fn parse_bc(s: &mut String, h: &mut DateHash) -> bool {
    static PAT: LazyLock<Regex> =
        LazyLock::new(|| re(r"(?i)\b(bc\b|bce\b|b\.c\.|b\.c\.e\.)".to_string()));
    subx(s, &PAT, h, |_, h| h.bc = true)
}

fn parse_frag(s: &mut String, h: &mut DateHash) -> bool {
    static PAT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\A\s*(\d{1,2})\s*\z".to_string()));
    subx(s, &PAT, h, |c, h| {
        let n = str2num(cap(c, 1).unwrap());
        if h.hour.is_some() && h.mday.is_none() && (1..=31).contains(&n) {
            h.mday = Some(n);
        }
        if h.mday.is_some() && h.hour.is_none() && (0..=24).contains(&n) {
            h.hour = Some(n);
        }
    })
}

const ALPHA: u8 = 1;
const DIGIT: u8 = 2;
const DASH: u8 = 4;
const DOT: u8 = 8;
const SLASH: u8 = 16;

fn has(s: &str, flags: u8) -> bool {
    let mut found = 0;
    for b in s.bytes() {
        if b.is_ascii_alphabetic() {
            found |= ALPHA;
        }
        if b.is_ascii_digit() {
            found |= DIGIT;
        }
        match b {
            b'-' => found |= DASH,
            b'.' => found |= DOT,
            b'/' => found |= SLASH,
            _ => {}
        }
    }
    found & flags == flags
}

/// `Date._parse(s, true)` for an ASCII string.
pub fn parse(input: &str) -> DateHash {
    static JUNK: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"[^-+',./:@[:alnum:]\[\]]+").unwrap());
    let mut s = JUNK.replace_all(input, " ").into_owned();
    let mut h = DateHash {
        comp: true,
        ..Default::default()
    };
    let s = &mut s;
    let h = &mut h;
    if has(s, ALPHA) {
        parse_day(s, h);
    }
    if has(s, DIGIT) {
        parse_time(s, h);
    }
    'ok: {
        if has(s, ALPHA | DIGIT) && (parse_eu(s, h) || parse_us(s, h)) {
            break 'ok;
        }
        if has(s, DIGIT | DASH) && parse_iso(s, h) {
            break 'ok;
        }
        if has(s, DIGIT | DOT) && parse_jis(s, h) {
            break 'ok;
        }
        if has(s, ALPHA | DIGIT | DASH) && parse_vms(s, h) {
            break 'ok;
        }
        if has(s, DIGIT | SLASH) && parse_sla(s, h) {
            break 'ok;
        }
        if has(s, DIGIT | DOT) && parse_dot(s, h) {
            break 'ok;
        }
        if has(s, DIGIT) && parse_iso2(s, h) {
            break 'ok;
        }
        if has(s, DIGIT) && parse_year(s, h) {
            break 'ok;
        }
        if has(s, ALPHA) && parse_mon(s, h) {
            break 'ok;
        }
        if has(s, DIGIT) && parse_mday(s, h) {
            break 'ok;
        }
        if has(s, DIGIT) && parse_ddd(s, h) {
            break 'ok;
        }
    }
    if has(s, ALPHA) {
        parse_bc(s, h);
    }
    if has(s, DIGIT) {
        parse_frag(s, h);
    }
    if std::mem::take(&mut h.bc) {
        if let Some(y) = h.cwyear {
            h.cwyear = Some(-y + 1);
        }
        if let Some(y) = h.year {
            h.year = Some(-y + 1);
        }
    }
    if std::mem::take(&mut h.comp) {
        let complete = |y: i64| if y >= 69 { y + 1900 } else { y + 2000 };
        if let Some(y) = h.cwyear.filter(|y| (0..=99).contains(y)) {
            h.cwyear = Some(complete(y));
        }
        if let Some(y) = h.year.filter(|y| (0..=99).contains(y)) {
            h.year = Some(complete(y));
        }
    }
    h.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    type Fields = [Option<i64>; 5];

    fn ymdhm(s: &str) -> Fields {
        let h = parse(s);
        [h.year, h.mon, h.mday, h.hour, h.min]
    }

    #[test]
    fn parses_as_date_parse() {
        assert_eq!(
            ymdhm("2014-08-08T20:03:49.008Zyyyy-MM-dd HH:mm:ss z"),
            [Some(2014), Some(8), Some(8), Some(20), Some(3)]
        );
        assert_eq!(
            parse("2014-08-08T20:03:49.008Zyyyy").zone.as_deref(),
            Some("Zyyyy")
        );
        assert_eq!(
            ymdhm("Sun, 15 Jan 2023 10:30:00 +0100"),
            [Some(2023), Some(1), Some(15), Some(10), Some(30)]
        );
        assert_eq!(
            ymdhm("January 15, 2023"),
            [Some(2023), Some(1), Some(15), None, None]
        );
        assert_eq!(
            ymdhm("20230115"),
            [Some(2023), Some(1), Some(15), None, None]
        );
        assert_eq!(
            ymdhm("2023/01/15"),
            [Some(2023), Some(1), Some(15), None, None]
        );
        assert_eq!(ymdhm("PT2H30M15S"), [None, None, None, Some(2), Some(30)]);
        assert_eq!(ymdhm("PT45M"), [None, None, Some(45), None, None]);
        assert_eq!(
            ymdhm("15/01/23"),
            [Some(2015), Some(1), Some(23), None, None]
        );
        assert_eq!(ymdhm("3pm"), [None, None, None, Some(15), None]);
    }
}
