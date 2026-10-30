//! Turns raw Spotify titles and artist names into comparable keys.

use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

/// What a title says once its version suffixes have been interpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleAnalysis {
    /// Normalized title without featuring credits nor version suffixes.
    pub core: String,
    /// Normalized names of the artists credited in the title (`feat.`, `with`).
    pub featured: Vec<String>,
    /// Normalized label of a different recording (`live`, `remix`, …), `None`
    /// for the original recording or one of its reissues.
    pub version: Option<String>,
    /// The version and reissue suffixes as written (`2009 Remaster`, `Live`).
    pub labels: Vec<String>,
}

enum Segment {
    Featuring(Vec<String>),
    SameRecording,
    Variant(String),
    Kept(String),
}

const REMIX: &[&str] = &["remix", "remixed", "mix"];

const FEATURING: &[&str] = &["feat", "ft", "featuring", "with"];

const DIFFERENT_RECORDING: &[&str] = &[
    "live",
    "acoustic",
    "unplugged",
    "remix",
    "remixed",
    "rmx",
    "mix",
    "demo",
    "instrumental",
    "edit",
    "karaoke",
    "acapella",
    "cappella",
    "rerecorded",
    "recorded",
    "reprise",
    "extended",
    "dub",
    "session",
    "sessions",
    "orchestral",
    "symphonic",
    "piano",
    "sped",
    "slowed",
    "reverb",
    "nightcore",
    "rehearsal",
    "alternate",
    "outtake",
];

const REMASTER: &[&str] = &[
    "remaster",
    "remastered",
    "remasterd",
    "remastering",
    "remasters",
];

const SAME_RECORDING: &[&str] = &[
    "deluxe",
    "anniversary",
    "expanded",
    "edition",
    "mono",
    "stereo",
    "single",
    "album",
    "version",
    "original",
    "bonus",
    "track",
    "explicit",
    "clean",
    "edited",
    "digital",
    "digitally",
    "special",
    "super",
    "collector",
    "collectors",
    "legacy",
    "reissue",
    "soundtrack",
    "motion",
    "picture",
    "lp",
    "ep",
    "hd",
];

const FILLER: &[&str] = &["and", "of", "the", "a"];

/// Lowercases, strips accents and apostrophes, spells `&` as `and` and turns
/// every other non-alphanumeric character into a word boundary.
pub fn normalize_text(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());
    for c in text.nfkd().filter(|c| !is_combining_mark(*c)) {
        match c {
            '\'' | '’' | '‘' | '`' | 'ʼ' => {}
            '&' => cleaned.push_str(" and "),
            c if c.is_alphanumeric() => cleaned.extend(c.to_lowercase()),
            _ => cleaned.push(' '),
        }
    }
    cleaned
        .split_whitespace()
        .map(|word| if word == "pt" { "part" } else { word })
        .collect::<Vec<_>>()
        .join(" ")
}

/// [`normalize_text`], without a leading `the`.
pub fn normalize_artist(name: &str) -> String {
    let normalized = normalize_text(name);
    match normalized.strip_prefix("the ") {
        Some(rest) => rest.to_owned(),
        None => normalized,
    }
}

/// Splits a title into its core, its featuring credits and its version.
///
/// Suffixes come either in brackets (`Song (Remastered 2009)`) or after a
/// spaced dash (`Song - 2011 Remaster`). A suffix that is not recognised
/// (`Part 2`, `Interlude`) stays in the core, since it tells songs apart.
pub fn analyze_title(title: &str) -> TitleAnalysis {
    let (outside, groups) = extract_bracket_groups(title);
    let mut pieces = split_on_dashes(&outside).into_iter();
    let head = pieces.next().unwrap_or_default();
    let (head, mut featured) = split_inline_featuring(&head);

    let mut kept = vec![head];
    let mut variants = Vec::new();
    let mut labels = Vec::new();
    for raw in groups
        .iter()
        .flat_map(|group| split_segment(group))
        .chain(pieces.flat_map(|piece| split_segment(&piece)))
    {
        match classify(&raw) {
            Segment::Featuring(names) => featured.extend(names),
            Segment::SameRecording => labels.push(raw),
            Segment::Variant(label) => {
                variants.push(label);
                labels.push(raw);
            }
            Segment::Kept(text) => kept.push(text),
        }
    }

    variants.sort();
    variants.dedup();
    featured.retain(|name| !name.is_empty());
    TitleAnalysis {
        core: normalize_text(&kept.join(" ")),
        featured,
        version: (!variants.is_empty()).then(|| variants.join(" ")),
        labels,
    }
}

fn extract_bracket_groups(title: &str) -> (String, Vec<String>) {
    let mut outside = String::new();
    let mut groups = Vec::new();
    let mut current = String::new();
    let mut depth = 0_usize;
    for c in title.chars() {
        match c {
            '(' | '[' => {
                if depth > 0 {
                    current.push(' ');
                }
                depth += 1;
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    groups.push(std::mem::take(&mut current));
                    outside.push(' ');
                } else {
                    current.push(' ');
                }
            }
            c if depth > 0 => current.push(c),
            c => outside.push(c),
        }
    }
    if !current.is_empty() {
        outside.push(' ');
        outside.push_str(&current);
    }
    (outside, groups)
}

fn split_on_dashes(text: &str) -> Vec<String> {
    text.replace('\u{a0}', " ")
        .replace(" – ", " - ")
        .replace(" — ", " - ")
        .split(" - ")
        .map(|piece| piece.trim().to_owned())
        .collect()
}

fn split_segment(segment: &str) -> Vec<String> {
    split_on_dashes(segment)
        .iter()
        .flat_map(|piece| piece.split(" / "))
        .map(|piece| piece.trim().to_owned())
        .filter(|piece| !piece.is_empty())
        .collect()
}

fn split_inline_featuring(head: &str) -> (String, Vec<String>) {
    let words: Vec<&str> = head.split_whitespace().collect();
    let position = words
        .iter()
        .skip(1)
        .position(|word| matches!(normalize_text(word).as_str(), "feat" | "ft" | "featuring"));
    match position {
        Some(index) => {
            let index = index + 1;
            (
                words[..index].join(" "),
                split_artist_list(&words[index + 1..].join(" ")),
            )
        }
        None => (head.to_owned(), Vec::new()),
    }
}

fn split_artist_list(names: &str) -> Vec<String> {
    names
        .replace(" and ", ",")
        .replace(" And ", ",")
        .replace(" x ", ",")
        .replace(" X ", ",")
        .split([',', '&'])
        .map(normalize_artist)
        .filter(|name| !name.is_empty())
        .collect()
}

fn is_year(word: &str) -> bool {
    word.len() == 4
        && word
            .parse::<u16>()
            .is_ok_and(|year| (1900..2100).contains(&year))
}

fn is_ordinal(word: &str) -> bool {
    ["st", "nd", "rd", "th"].iter().any(|suffix| {
        word.strip_suffix(suffix)
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// `2018 Remix`: a dated remix of the artist's own record, issued like a
/// remaster, unlike `DJ Name Remix`, which is someone else's take.
fn is_year_remix(words: &[&str]) -> bool {
    let significant: Vec<&str> = words
        .iter()
        .copied()
        .filter(|word| {
            !SAME_RECORDING.contains(word) && !FILLER.contains(word) && !REMASTER.contains(word)
        })
        .collect();
    matches!(significant.as_slice(), [year, mix] if is_year(year) && REMIX.contains(mix))
}

fn classify(raw: &str) -> Segment {
    let normalized = normalize_text(raw);
    let words: Vec<&str> = normalized.split_whitespace().collect();
    let Some(first) = words.first() else {
        return Segment::SameRecording;
    };

    if FEATURING.contains(first) {
        let names = raw
            .trim()
            .split_once(char::is_whitespace)
            .map_or("", |(_, rest)| rest);
        return Segment::Featuring(split_artist_list(names));
    }
    if *first == "from" {
        return Segment::SameRecording;
    }

    if is_year_remix(&words) {
        return Segment::SameRecording;
    }

    let has = |set: &[&str]| words.iter().any(|word| set.contains(word));
    let is_mastering_mix = has(&["mono", "stereo", "original"]);
    let is_edition = has(&["edition"]);
    let is_different = |word: &&str| match *word {
        "mix" => !is_mastering_mix,
        "extended" => !is_edition,
        word => DIFFERENT_RECORDING.contains(&word),
    };
    let possessive_version = raw.to_lowercase().replace('’', "'").contains("'s version");

    if possessive_version || words.iter().any(is_different) {
        let remastered = has(REMASTER);
        let label: Vec<&str> = words
            .iter()
            .copied()
            .filter(|word| {
                let reissue = REMASTER.contains(word)
                    || SAME_RECORDING.contains(word)
                    || FILLER.contains(word)
                    || (remastered && is_year(word));
                !reissue
            })
            .collect();
        let label = label.join(" ");
        return Segment::Variant(if label.is_empty() { normalized } else { label });
    }

    let is_reissue_word = |word: &&str| {
        REMASTER.contains(word)
            || SAME_RECORDING.contains(word)
            || is_year(word)
            || is_ordinal(word)
    };
    let all_reissue = words.iter().all(|word| {
        is_reissue_word(word)
            || FILLER.contains(word)
            || *word == "mix"
            || (*word == "extended" && is_edition)
    });
    if all_reissue && words.iter().any(is_reissue_word) {
        Segment::SameRecording
    } else {
        Segment::Kept(raw.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core(title: &str) -> String {
        analyze_title(title).core
    }

    fn version(title: &str) -> Option<String> {
        analyze_title(title).version
    }

    #[test]
    fn normalize_text_folds_case_accents_and_punctuation() {
        assert_eq!(normalize_text("Beyoncé – Déjà Vu!"), "beyonce deja vu");
        assert_eq!(normalize_text("  Mr.   Brightside "), "mr brightside");
        assert_eq!(normalize_text("Don't Stop Me Now"), "dont stop me now");
        assert_eq!(normalize_text("Don’t Stop Me Now"), "dont stop me now");
    }

    #[test]
    fn normalize_text_spells_ampersand_and_part() {
        assert_eq!(normalize_text("Rock & Roll"), "rock and roll");
        assert_eq!(normalize_text("Rock&Roll"), "rock and roll");
        assert_eq!(normalize_text("Pt. 2"), "part 2");
    }

    #[test]
    fn normalize_text_keeps_non_latin_scripts() {
        assert_eq!(normalize_text("夜に駆ける"), "夜に駆ける");
        assert_eq!(normalize_text("Кино"), "кино");
    }

    #[test]
    fn normalize_artist_drops_leading_the() {
        assert_eq!(normalize_artist("The Beatles"), "beatles");
        assert_eq!(
            normalize_artist("Theory of a Deadman"),
            "theory of a deadman"
        );
        assert_eq!(normalize_artist("Simon & Garfunkel"), "simon and garfunkel");
    }

    #[test]
    fn remaster_suffixes_are_the_same_recording() {
        for title in [
            "Let It Be - Remastered 2009",
            "Let It Be - 2009 Remaster",
            "Let It Be (Remastered 2009)",
            "Let It Be (2009 Remaster)",
            "Let It Be - Remastered",
            "Let It Be [Remastered Version]",
            "Let It Be - Digital Remaster",
            "Let It Be – 2015 Remastered",
        ] {
            let analysis = analyze_title(title);
            assert_eq!(analysis.core, "let it be", "{title}");
            assert_eq!(analysis.version, None, "{title}");
        }
    }

    #[test]
    fn edition_and_channel_suffixes_are_the_same_recording() {
        for title in [
            "Song - Deluxe Edition",
            "Song (Deluxe)",
            "Song - 25th Anniversary Edition",
            "Song (Expanded Edition)",
            "Song - Mono",
            "Song - Stereo",
            "Song - 2009 Stereo Mix",
            "Song - Mono Version",
            "Song - Single Version",
            "Song (Album Version)",
            "Song - Original Mix",
            "Song - Bonus Track",
            "Song (From \"Some Movie\")",
            "Song - Extended Edition",
        ] {
            let analysis = analyze_title(title);
            assert_eq!(analysis.core, "song", "{title}");
            assert_eq!(analysis.version, None, "{title}");
        }
    }

    #[test]
    fn different_recordings_carry_a_version() {
        for (title, expected) in [
            ("Song - Live", "live"),
            ("Song (Live)", "live"),
            ("Song - Live at Wembley", "live at wembley"),
            ("Song - Acoustic", "acoustic"),
            ("Song (Acoustic Version)", "acoustic"),
            ("Song - Radio Edit", "radio edit"),
            ("Song - Single Edit", "edit"),
            ("Song - Demo", "demo"),
            ("Song (Instrumental)", "instrumental"),
            ("Song - Some DJ Remix", "some dj remix"),
            ("Song - Extended Mix", "extended mix"),
            ("Song (Taylor's Version)", "taylors"),
            ("Song (Taylor’s Version)", "taylors"),
            ("Song - Re-Recorded", "re recorded"),
            ("Song - Sped Up", "sped up"),
        ] {
            let analysis = analyze_title(title);
            assert_eq!(analysis.core, "song", "{title}");
            assert_eq!(analysis.version.as_deref(), Some(expected), "{title}");
        }
    }

    #[test]
    fn year_prefixed_remix_is_the_same_recording() {
        for title in [
            "Sheep - 2018 Remix",
            "Sheep (2018 Remix)",
            "Sheep - 2018 Remixed Version",
            "Sheep - 2019 Mix",
        ] {
            let analysis = analyze_title(title);
            assert_eq!(analysis.core, "sheep", "{title}");
            assert_eq!(analysis.version, None, "{title}");
        }
    }

    #[test]
    fn remix_without_a_leading_year_stays_a_variant() {
        assert_eq!(version("Sheep - Remix").as_deref(), Some("remix"));
        assert_eq!(
            version("Sheep - Some DJ 2018 Remix").as_deref(),
            Some("some dj 2018 remix")
        );
        assert_eq!(version("Sheep - Remix 2018").as_deref(), Some("remix 2018"));
    }

    #[test]
    fn labels_keep_the_suffixes_as_written() {
        assert_eq!(
            analyze_title("Song (feat. B) - 2011 Remaster").labels,
            ["2011 Remaster"]
        );
        assert_eq!(
            analyze_title("Song (Live) - Remastered").labels,
            ["Live", "Remastered"]
        );
        assert_eq!(analyze_title("Song (Part 2)").labels, Vec::<String>::new());
    }

    #[test]
    fn remaster_inside_a_variant_does_not_change_its_label() {
        assert_eq!(
            version("Song (Live - Remastered 2011)").as_deref(),
            Some("live")
        );
        assert_eq!(version("Song - Live / Remastered").as_deref(), Some("live"));
        assert_eq!(
            version("Song - Live 2011 Remaster").as_deref(),
            Some("live")
        );
        assert_eq!(version("Song - Live 1986").as_deref(), Some("live 1986"));
    }

    #[test]
    fn unknown_suffixes_stay_in_the_core() {
        assert_eq!(core("Symphony (Part 2)"), "symphony part 2");
        assert_eq!(core("Symphony - Pt. 2"), "symphony part 2");
        assert_eq!(core("Song - Interlude"), "song interlude");
        assert_eq!(version("Song - Interlude"), None);
        assert_ne!(
            core("Another Brick in the Wall, Pt. 1"),
            core("Another Brick in the Wall, Pt. 2")
        );
    }

    #[test]
    fn hyphen_without_spaces_is_part_of_the_title() {
        assert_eq!(core("Re-Live"), "re live");
        assert_eq!(version("Re-Live"), None);
    }

    #[test]
    fn a_title_made_of_a_version_word_is_kept() {
        assert_eq!(core("Live"), "live");
        assert_eq!(version("Live"), None);
        assert_eq!(core("Remix"), "remix");
    }

    #[test]
    fn featuring_in_brackets_is_extracted() {
        let analysis = analyze_title("Song (feat. Artist B & Artist C)");
        assert_eq!(analysis.core, "song");
        assert_eq!(analysis.featured, ["artist b", "artist c"]);
        assert_eq!(analysis.version, None);

        let analysis = analyze_title("Song [ft. B, C and D]");
        assert_eq!(analysis.featured, ["b", "c", "d"]);

        let analysis = analyze_title("Song (with The Weeknd)");
        assert_eq!(analysis.core, "song");
        assert_eq!(analysis.featured, ["weeknd"]);
    }

    #[test]
    fn inline_featuring_is_extracted() {
        let analysis = analyze_title("Song feat. Artist B");
        assert_eq!(analysis.core, "song");
        assert_eq!(analysis.featured, ["artist b"]);

        let analysis = analyze_title("Song featuring B - Remastered 2010");
        assert_eq!(analysis.core, "song");
        assert_eq!(analysis.featured, ["b"]);
        assert_eq!(analysis.version, None);
    }

    #[test]
    fn with_outside_brackets_belongs_to_the_title() {
        assert_eq!(core("Dancing with Myself"), "dancing with myself");
        assert!(analyze_title("Dancing with Myself").featured.is_empty());
    }

    #[test]
    fn featuring_and_version_combine() {
        let analysis = analyze_title("Song (feat. B) - Live");
        assert_eq!(analysis.core, "song");
        assert_eq!(analysis.featured, ["b"]);
        assert_eq!(analysis.version.as_deref(), Some("live"));
    }

    #[test]
    fn several_variant_suffixes_are_sorted() {
        assert_eq!(
            version("Song (Acoustic) - Live").as_deref(),
            Some("acoustic live")
        );
        assert_eq!(
            version("Song - Live (Acoustic)").as_deref(),
            Some("acoustic live")
        );
    }

    #[test]
    fn unbalanced_brackets_do_not_lose_text() {
        assert_eq!(core("Song (unfinished"), "song unfinished");
        assert_eq!(core("Song) odd"), "song odd");
    }

    #[test]
    fn empty_title() {
        let analysis = analyze_title("");
        assert_eq!(analysis.core, "");
        assert_eq!(analysis.version, None);
        assert!(analysis.featured.is_empty());
    }
}
