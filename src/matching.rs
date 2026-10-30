//! Duplicate detection among saved tracks, free of any I/O.

mod normalize;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use normalize::{analyze_title, normalize_artist};

/// A saved track, as far as duplicate detection is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    /// Album release date as Spotify gives it: `YYYY`, `YYYY-MM` or `YYYY-MM-DD`.
    pub release_date: Option<String>,
    pub duration: Duration,
    pub isrc: Option<String>,
    /// When the track was saved, as an RFC 3339 timestamp.
    pub added_at: String,
}

/// How sure a [`Group`] is to hold copies of one recording, most certain first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Confidence {
    /// Every track carries the same ISRC.
    Certain,
    /// Same title and artist once normalized, same version, close durations.
    Probable,
    /// Same title and artist, but different recordings (live, remix, …).
    Variant,
}

/// Tracks that look like the same song.
#[derive(Debug, PartialEq, Eq)]
pub struct Group<'a> {
    pub confidence: Confidence,
    /// Sorted by release date, then by the date they were saved.
    pub tracks: Vec<&'a Track>,
}

struct Key {
    core: String,
    primary_artist: String,
    artists: BTreeSet<String>,
    version: Option<String>,
    isrc: Option<String>,
}

impl Key {
    fn new(track: &Track) -> Self {
        let title = analyze_title(&track.title);
        let primary_artist = track
            .artists
            .first()
            .map(|name| normalize_artist(name))
            .unwrap_or_default();
        let artists = track
            .artists
            .iter()
            .map(|name| normalize_artist(name))
            .chain(title.featured)
            .filter(|name| !name.is_empty())
            .collect();
        let isrc = track
            .isrc
            .as_deref()
            .map(|isrc| {
                isrc.chars()
                    .filter(char::is_ascii_alphanumeric)
                    .collect::<String>()
                    .to_ascii_uppercase()
            })
            .filter(|isrc| !isrc.is_empty());
        Self {
            core: title.core,
            primary_artist,
            artists,
            version: title.version,
            isrc,
        }
    }

    /// Each primary artist is credited on the other track, which reconciles a
    /// guest moved between the title and the artist list, and a different
    /// credit order, while keeping covers apart. When both tracks credit
    /// someone else as well, one of those must be shared: the same composer
    /// played by two orchestras is two recordings.
    fn same_artist(&self, other: &Self) -> bool {
        if self.primary_artist.is_empty()
            || other.primary_artist.is_empty()
            || !self.artists.contains(&other.primary_artist)
            || !other.artists.contains(&self.primary_artist)
        {
            return false;
        }
        let primaries = [&self.primary_artist, &other.primary_artist];
        let others = |key: &Self| -> BTreeSet<String> {
            key.artists
                .iter()
                .filter(|name| !primaries.contains(name))
                .cloned()
                .collect()
        };
        let (mine, theirs) = (others(self), others(other));
        mine.is_empty() || theirs.is_empty() || !mine.is_disjoint(&theirs)
    }
}

struct DisjointSet(Vec<usize>);

impl DisjointSet {
    fn new(size: usize) -> Self {
        Self((0..size).collect())
    }

    fn find(&mut self, mut index: usize) -> usize {
        while self.0[index] != index {
            self.0[index] = self.0[self.0[index]];
            index = self.0[index];
        }
        index
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b)] = a.min(b);
        }
    }

    fn components(&mut self, members: impl IntoIterator<Item = usize>) -> Vec<Vec<usize>> {
        let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for member in members {
            let root = self.find(member);
            components.entry(root).or_default().push(member);
        }
        components.into_values().collect()
    }
}

/// Groups `tracks` that look like the same song, most certain groups first.
///
/// Durations further apart than `tolerance` mean different songs that happen
/// to share a title and an artist (`Intro`, `Untitled`), so such tracks are
/// never reported as duplicates.
pub fn find_groups(tracks: &[Track], tolerance: Duration) -> Vec<Group<'_>> {
    let keys: Vec<Key> = tracks.iter().map(Key::new).collect();
    let mut duplicates = DisjointSet::new(tracks.len());

    let mut by_isrc: HashMap<&str, usize> = HashMap::new();
    for (index, key) in keys.iter().enumerate() {
        if let Some(isrc) = &key.isrc {
            let first = *by_isrc.entry(isrc).or_insert(index);
            duplicates.union(first, index);
        }
    }

    let mut by_title: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, key) in keys.iter().enumerate() {
        if !key.core.is_empty() {
            by_title.entry(&key.core).or_default().push(index);
        }
    }

    let mut groups = Vec::new();
    for bucket in by_title.values().filter(|bucket| bucket.len() > 1) {
        let mut songs = DisjointSet::new(tracks.len());
        for (position, &a) in bucket.iter().enumerate() {
            for &b in &bucket[position + 1..] {
                if !keys[a].same_artist(&keys[b]) {
                    continue;
                }
                songs.union(a, b);
                if keys[a].version == keys[b].version
                    && tracks[a].duration.abs_diff(tracks[b].duration) <= tolerance
                {
                    duplicates.union(a, b);
                }
            }
        }
        for song in songs.components(bucket.iter().copied()) {
            let versions: BTreeSet<_> = song.iter().map(|&index| &keys[index].version).collect();
            if versions.len() > 1 {
                groups.push(group(tracks, Confidence::Variant, &song));
            }
        }
    }

    for members in duplicates.components(0..tracks.len()) {
        if members.len() < 2 {
            continue;
        }
        let first_isrc = &keys[members[0]].isrc;
        let confidence = if first_isrc.is_some()
            && members.iter().all(|&index| keys[index].isrc == *first_isrc)
        {
            Confidence::Certain
        } else {
            Confidence::Probable
        };
        groups.push(group(tracks, confidence, &members));
    }

    groups.sort_by_cached_key(|group| {
        let first = group.tracks[0];
        (
            group.confidence,
            normalize::normalize_text(&first.title),
            first.id.clone(),
        )
    });
    groups
}

fn group<'a>(tracks: &'a [Track], confidence: Confidence, members: &[usize]) -> Group<'a> {
    let mut tracks: Vec<&Track> = members.iter().map(|&index| &tracks[index]).collect();
    tracks.sort_by(|a, b| {
        (&a.release_date, &a.added_at, &a.id).cmp(&(&b.release_date, &b.added_at, &b.id))
    });
    Group { confidence, tracks }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOLERANCE: Duration = Duration::from_secs(10);

    fn track(id: &str, title: &str, artists: &[&str], seconds: u64) -> Track {
        Track {
            id: id.to_owned(),
            title: title.to_owned(),
            artists: artists.iter().map(|&name| name.to_owned()).collect(),
            album: format!("Album of {id}"),
            release_date: None,
            duration: Duration::from_secs(seconds),
            isrc: None,
            added_at: "2024-01-01T00:00:00Z".to_owned(),
        }
    }

    fn with_isrc(mut track: Track, isrc: &str) -> Track {
        track.isrc = Some(isrc.to_owned());
        track
    }

    fn with_release(mut track: Track, date: &str) -> Track {
        track.release_date = Some(date.to_owned());
        track
    }

    fn summary(groups: &[Group<'_>]) -> Vec<(Confidence, Vec<String>)> {
        groups
            .iter()
            .map(|group| {
                let mut ids: Vec<String> = group.tracks.iter().map(|t| t.id.clone()).collect();
                ids.sort();
                (group.confidence, ids)
            })
            .collect()
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|&id| id.to_owned()).collect()
    }

    #[test]
    fn same_isrc_is_certain_whatever_the_names() {
        let tracks = [
            with_isrc(track("a", "Song", &["Artist"], 200), "GBAYE0601498"),
            with_isrc(
                track("b", "Totally Different", &["Other"], 260),
                "gb-aye-06-01498",
            ),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Certain, ids(&["a", "b"]))]
        );
    }

    #[test]
    fn remaster_with_a_new_isrc_is_probable() {
        let tracks = [
            with_isrc(
                track("a", "Let It Be", &["The Beatles"], 243),
                "GBAYE0601498",
            ),
            with_isrc(
                track("b", "Let It Be - Remastered 2009", &["The Beatles"], 241),
                "GBAYE0901234",
            ),
            with_isrc(
                track("c", "Let It Be (2015 Remaster)", &["Beatles"], 245),
                "GBAYE1501234",
            ),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Probable, ids(&["a", "b", "c"]))]
        );
    }

    #[test]
    fn isrc_and_name_matches_merge_into_one_probable_group() {
        let tracks = [
            with_isrc(track("a", "Song", &["Artist"], 200), "USAAA0000001"),
            with_isrc(
                track("b", "Song - Remastered", &["Artist"], 201),
                "USAAA0000001",
            ),
            track("c", "Song (Deluxe Edition)", &["Artist"], 199),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Probable, ids(&["a", "b", "c"]))]
        );
    }

    #[test]
    fn live_and_studio_are_variants_never_duplicates() {
        let tracks = [
            track("a", "Song", &["Artist"], 200),
            track("b", "Song - Live", &["Artist"], 205),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Variant, ids(&["a", "b"]))]
        );
    }

    #[test]
    fn studio_duplicates_and_a_live_take() {
        let tracks = [
            track("a", "Song", &["Artist"], 200),
            track("b", "Song - 2011 Remaster", &["Artist"], 201),
            track("c", "Song - Live at Wembley", &["Artist"], 260),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [
                (Confidence::Probable, ids(&["a", "b"])),
                (Confidence::Variant, ids(&["a", "b", "c"])),
            ]
        );
    }

    #[test]
    fn two_copies_of_the_same_live_take_are_probable() {
        let tracks = [
            track("a", "Song - Live", &["Artist"], 300),
            track("b", "Song (Live)", &["Artist"], 302),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Probable, ids(&["a", "b"]))]
        );
    }

    #[test]
    fn different_remixes_are_variants() {
        let tracks = [
            track("a", "Song - DJ One Remix", &["Artist"], 300),
            track("b", "Song - DJ Two Remix", &["Artist"], 300),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Variant, ids(&["a", "b"]))]
        );
    }

    #[test]
    fn taylors_version_is_a_variant() {
        let tracks = [
            track("a", "Love Story", &["Taylor Swift"], 235),
            track("b", "Love Story (Taylor's Version)", &["Taylor Swift"], 236),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Variant, ids(&["a", "b"]))]
        );
    }

    #[test]
    fn featuring_in_title_matches_featuring_in_artists() {
        let tracks = [
            track("a", "Song (feat. Guest)", &["Artist"], 200),
            track("b", "Song", &["Artist", "Guest"], 200),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Probable, ids(&["a", "b"]))]
        );
    }

    #[test]
    fn credit_order_does_not_matter() {
        let tracks = [
            track("a", "Song", &["Artist A", "Artist B"], 200),
            track("b", "Song", &["Artist B", "Artist A"], 200),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Probable, ids(&["a", "b"]))]
        );
    }

    #[test]
    fn same_piece_by_different_performers_is_not_a_duplicate() {
        let title = "Symphony No. 5 in C Minor, Op. 67: I. Allegro con brio";
        let tracks = [
            track(
                "a",
                title,
                &[
                    "Ludwig van Beethoven",
                    "Berliner Philharmoniker",
                    "Herbert von Karajan",
                ],
                440,
            ),
            track(
                "b",
                title,
                &[
                    "Ludwig van Beethoven",
                    "Wiener Philharmoniker",
                    "Carlos Kleiber",
                ],
                445,
            ),
            track(
                "c",
                title,
                &[
                    "Ludwig van Beethoven",
                    "Berliner Philharmoniker",
                    "Herbert von Karajan",
                ],
                441,
            ),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Probable, ids(&["a", "c"]))]
        );
    }

    #[test]
    fn different_guests_are_different_recordings() {
        let tracks = [
            track("a", "Song (feat. Guest One)", &["Artist"], 200),
            track("b", "Song", &["Artist", "Guest Two"], 200),
        ];
        assert!(find_groups(&tracks, TOLERANCE).is_empty());
    }

    #[test]
    fn cover_by_another_artist_is_not_a_duplicate() {
        let tracks = [
            track("a", "Hallelujah", &["Leonard Cohen"], 279),
            track("b", "Hallelujah", &["Jeff Buckley"], 280),
        ];
        assert!(find_groups(&tracks, TOLERANCE).is_empty());
    }

    #[test]
    fn cover_featuring_the_original_artist_is_not_a_duplicate() {
        let tracks = [
            track("a", "Song", &["Original"], 200),
            track("b", "Song (feat. Original)", &["Cover Band"], 200),
        ];
        assert!(find_groups(&tracks, TOLERANCE).is_empty());
    }

    #[test]
    fn homonyms_with_distant_durations_are_not_duplicates() {
        let tracks = [
            track("a", "Intro", &["Artist"], 60),
            track("b", "Intro", &["Artist"], 95),
        ];
        assert!(find_groups(&tracks, TOLERANCE).is_empty());
    }

    #[test]
    fn duration_tolerance_is_inclusive() {
        let tracks = [
            track("a", "Song", &["Artist"], 200),
            track("b", "Song", &["Artist"], 210),
            track("c", "Song", &["Artist"], 221),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Probable, ids(&["a", "b"]))]
        );
        assert_eq!(
            summary(&find_groups(&tracks, Duration::from_secs(11))),
            [(Confidence::Probable, ids(&["a", "b", "c"]))]
        );
    }

    #[test]
    fn accents_case_and_punctuation_are_ignored() {
        let tracks = [
            track("a", "Déjà Vu", &["Beyoncé"], 240),
            track("b", "deja vu!", &["BEYONCE"], 241),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Probable, ids(&["a", "b"]))]
        );
    }

    #[test]
    fn ampersand_and_and_are_the_same() {
        let tracks = [
            track("a", "Rock & Roll", &["Simon & Garfunkel"], 180),
            track("b", "Rock and Roll", &["Simon and Garfunkel"], 181),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Probable, ids(&["a", "b"]))]
        );
    }

    #[test]
    fn different_parts_are_different_songs() {
        let tracks = [
            track(
                "a",
                "Another Brick in the Wall, Pt. 1",
                &["Pink Floyd"],
                190,
            ),
            track(
                "b",
                "Another Brick in the Wall, Pt. 2",
                &["Pink Floyd"],
                195,
            ),
        ];
        assert!(find_groups(&tracks, TOLERANCE).is_empty());
    }

    #[test]
    fn unknown_subtitle_in_brackets_is_part_of_the_title() {
        let tracks = [
            track(
                "a",
                "Country Roads (Take Me Home) - Remastered",
                &["John Denver"],
                190,
            ),
            track("b", "Country Roads (Take Me Home)", &["John Denver"], 191),
            track("c", "Country Roads", &["John Denver"], 191),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Probable, ids(&["a", "b"]))]
        );
    }

    #[test]
    fn tracks_without_artist_never_match_by_name() {
        let tracks = [track("a", "Song", &[], 200), track("b", "Song", &[], 200)];
        assert!(find_groups(&tracks, TOLERANCE).is_empty());
    }

    #[test]
    fn unrelated_tracks_give_nothing() {
        let tracks = [
            track("a", "One", &["Artist"], 200),
            track("b", "Two", &["Artist"], 200),
        ];
        assert!(find_groups(&tracks, TOLERANCE).is_empty());
        assert!(find_groups(&[], TOLERANCE).is_empty());
    }

    #[test]
    fn groups_are_sorted_by_confidence_and_tracks_by_release_date() {
        let tracks = [
            track("v1", "Alpha - Live", &["Artist"], 100),
            track("v2", "Alpha", &["Artist"], 100),
            with_release(
                track("p2", "Beta - 2011 Remaster", &["Artist"], 100),
                "2011-05-02",
            ),
            with_release(track("p1", "Beta", &["Artist"], 100), "1979"),
            with_isrc(track("c1", "Gamma", &["Artist"], 100), "USAAA0000001"),
            with_isrc(track("c2", "Gamma", &["Artist"], 100), "USAAA0000001"),
        ];
        let groups = find_groups(&tracks, TOLERANCE);
        let confidences: Vec<Confidence> = groups.iter().map(|group| group.confidence).collect();
        assert_eq!(
            confidences,
            [
                Confidence::Certain,
                Confidence::Probable,
                Confidence::Variant
            ]
        );
        let probable: Vec<&str> = groups[1].tracks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(probable, ["p1", "p2"]);
    }
}
