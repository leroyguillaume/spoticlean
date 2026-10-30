//! Renders duplicate groups as the text `spoticlean saved` prints.

use std::fmt::Write as _;
use std::time::Duration;

use crate::matching::{Confidence, Group, Track};

/// Formats every group, then a one-line summary.
pub fn render(groups: &[Group<'_>], scanned: usize) -> String {
    let mut out = String::new();
    for group in groups {
        let first = group.tracks[0];
        let _ = writeln!(
            out,
            "[{}] {} - {}",
            label(group.confidence),
            first.title,
            first.artists.join(", ")
        );
        for track in &group.tracks {
            let _ = writeln!(out, "  - {}", describe(track));
        }
        out.push('\n');
    }
    out.push_str(&summary(groups, scanned));
    out.push('\n');
    out
}

fn label(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Certain => "certain",
        Confidence::Probable => "probable",
        Confidence::Variant => "variant",
    }
}

fn describe(track: &Track) -> String {
    format!(
        "{} | {} | {} | released {} | {} | added {}",
        track.title,
        track.artists.join(", "),
        track.album,
        track.release_date.as_deref().unwrap_or("unknown"),
        format_duration(track.duration),
        track.added_at.get(..10).unwrap_or(&track.added_at),
    )
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn summary(groups: &[Group<'_>], scanned: usize) -> String {
    let count = |confidence| {
        groups
            .iter()
            .filter(|group| group.confidence == confidence)
            .count()
    };
    let redundant: usize = groups
        .iter()
        .filter(|group| group.confidence != Confidence::Variant)
        .map(|group| group.tracks.len() - 1)
        .sum();
    format!(
        "{scanned} liked songs scanned: {} certain and {} probable duplicate groups \
         ({redundant} redundant tracks), {} variant groups",
        count(Confidence::Certain),
        count(Confidence::Probable),
        count(Confidence::Variant),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, title: &str, release_date: Option<&str>, seconds: u64) -> Track {
        Track {
            id: id.to_owned(),
            title: title.to_owned(),
            artists: vec!["Artist".to_owned(), "Guest".to_owned()],
            album: "Album".to_owned(),
            release_date: release_date.map(str::to_owned),
            duration: Duration::from_secs(seconds),
            isrc: None,
            added_at: "2023-04-05T06:07:08Z".to_owned(),
        }
    }

    #[test]
    fn format_duration_pads_seconds() {
        assert_eq!(format_duration(Duration::from_secs(65)), "1:05");
        assert_eq!(format_duration(Duration::from_secs(600)), "10:00");
        assert_eq!(format_duration(Duration::from_millis(59_999)), "0:59");
    }

    #[test]
    fn render_lists_groups_and_summary() {
        let a = track("a", "Song", Some("1979"), 200);
        let b = track("b", "Song - 2011 Remaster", Some("2011-05-02"), 201);
        let c = track("c", "Song - Live", None, 260);
        let groups = [
            Group {
                confidence: Confidence::Probable,
                tracks: vec![&a, &b],
            },
            Group {
                confidence: Confidence::Variant,
                tracks: vec![&a, &b, &c],
            },
        ];
        let expected = "\
[probable] Song - Artist, Guest
  - Song | Artist, Guest | Album | released 1979 | 3:20 | added 2023-04-05
  - Song - 2011 Remaster | Artist, Guest | Album | released 2011-05-02 | 3:21 | added 2023-04-05

[variant] Song - Artist, Guest
  - Song | Artist, Guest | Album | released 1979 | 3:20 | added 2023-04-05
  - Song - 2011 Remaster | Artist, Guest | Album | released 2011-05-02 | 3:21 | added 2023-04-05
  - Song - Live | Artist, Guest | Album | released unknown | 4:20 | added 2023-04-05

12 liked songs scanned: 0 certain and 1 probable duplicate groups (1 redundant tracks), 1 variant groups
";
        assert_eq!(render(&groups, 12), expected);
    }

    #[test]
    fn render_without_groups_prints_the_summary() {
        assert_eq!(
            render(&[], 3),
            "3 liked songs scanned: 0 certain and 0 probable duplicate groups \
             (0 redundant tracks), 0 variant groups\n"
        );
    }
}
