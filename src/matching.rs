//! Duplicate detection among saved tracks, free of any I/O.

mod normalize;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::time::Duration;

use normalize::{analyze_title, normalize_artist};
use sha2::{Digest, Sha256};

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
    /// Same title and credits once normalized, same version, close durations.
    Probable,
    /// Same song, but different recordings (live, remix, new guests, …).
    Variant,
}

/// Tracks that look like the same song.
#[derive(Debug, PartialEq, Eq)]
pub struct Group<'a> {
    /// Derived from the tracks' names, not from their order, so that the same
    /// group gets the same key on the next scan.
    pub key: String,
    pub confidence: Confidence,
    /// Sorted by release date, then by the date they were saved.
    pub tracks: Vec<&'a Track>,
}

/// The suffixes of `title` that name its version or reissue, as written
/// (`2009 Remaster`, `Live at Wembley`), `None` for a bare title.
pub fn version_label(title: &str) -> Option<String> {
    let labels = analyze_title(title).labels;
    (!labels.is_empty()).then(|| labels.join(" · "))
}

struct Fingerprint {
    core: String,
    primary_artist: String,
    /// Everyone credited, in the artist list or in the title.
    artists: BTreeSet<String>,
    version: Option<String>,
    isrc: Option<String>,
}

impl Fingerprint {
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
    fn same_song(&self, other: &Self) -> bool {
        if self.primary_artist.is_empty()
            || other.primary_artist.is_empty()
            || !self.artists.contains(&other.primary_artist)
            || !other.artists.contains(&self.primary_artist)
        {
            return false;
        }
        let primaries = [&self.primary_artist, &other.primary_artist];
        let others = |fingerprint: &Self| -> BTreeSet<String> {
            fingerprint
                .artists
                .iter()
                .filter(|name| !primaries.contains(name))
                .cloned()
                .collect()
        };
        let (mine, theirs) = (others(self), others(other));
        mine.is_empty() || theirs.is_empty() || !mine.is_disjoint(&theirs)
    }

    /// What tells two recordings of one song apart: the version, and who is
    /// credited. A guest credited on one track only means a re-recording.
    fn recording(&self) -> (&Option<String>, &BTreeSet<String>) {
        (&self.version, &self.artists)
    }

    /// Identifies the recording by name, or by ISRC for a track whose name
    /// says nothing.
    fn recording_identity(&self, track: &Track) -> String {
        if self.core.is_empty() || self.primary_artist.is_empty() {
            return match &self.isrc {
                Some(isrc) => format!("isrc:{isrc}"),
                None => format!("id:{}", track.id),
            };
        }
        let credits: Vec<&str> = self.artists.iter().map(String::as_str).collect();
        format!(
            "name:{}\u{1f}{}\u{1f}{}",
            self.core,
            self.version.as_deref().unwrap_or_default(),
            credits.join("\u{1e}")
        )
    }

    fn song_identity(&self) -> String {
        format!("name:{}\u{1f}{}", self.core, self.primary_artist)
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

/// A group before its key is final: `identity` is what the key hashes.
struct Draft {
    confidence: Confidence,
    identity: String,
    members: Vec<usize>,
}

/// Groups `tracks` that look like the same song, most certain groups first.
///
/// Durations further apart than `tolerance` mean different songs that happen
/// to share a title and an artist (`Intro`, `Untitled`), so such tracks are
/// never reported as duplicates.
pub fn find_groups(tracks: &[Track], tolerance: Duration) -> Vec<Group<'_>> {
    let fingerprints: Vec<Fingerprint> = tracks.iter().map(Fingerprint::new).collect();
    let mut duplicates = DisjointSet::new(tracks.len());

    let mut by_isrc: HashMap<&str, usize> = HashMap::new();
    for (index, fingerprint) in fingerprints.iter().enumerate() {
        if let Some(isrc) = &fingerprint.isrc {
            let first = *by_isrc.entry(isrc).or_insert(index);
            duplicates.union(first, index);
        }
    }

    let mut by_title: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, fingerprint) in fingerprints.iter().enumerate() {
        if !fingerprint.core.is_empty() {
            by_title.entry(&fingerprint.core).or_default().push(index);
        }
    }

    let mut drafts = Vec::new();
    for bucket in by_title.values().filter(|bucket| bucket.len() > 1) {
        let mut songs = DisjointSet::new(tracks.len());
        for (position, &a) in bucket.iter().enumerate() {
            for &b in &bucket[position + 1..] {
                if !fingerprints[a].same_song(&fingerprints[b]) {
                    continue;
                }
                songs.union(a, b);
                if fingerprints[a].recording() == fingerprints[b].recording()
                    && tracks[a].duration.abs_diff(tracks[b].duration) <= tolerance
                {
                    duplicates.union(a, b);
                }
            }
        }
        for song in songs.components(bucket.iter().copied()) {
            let recordings: BTreeSet<_> = song
                .iter()
                .map(|&index| fingerprints[index].recording())
                .collect();
            if recordings.len() > 1 {
                drafts.push(Draft {
                    confidence: Confidence::Variant,
                    identity: smallest(
                        song.iter()
                            .map(|&index| fingerprints[index].song_identity()),
                    ),
                    members: song,
                });
            }
        }
    }

    for members in duplicates.components(0..tracks.len()) {
        if members.len() < 2 {
            continue;
        }
        let first_isrc = &fingerprints[members[0]].isrc;
        let confidence = if first_isrc.is_some()
            && members
                .iter()
                .all(|&index| fingerprints[index].isrc == *first_isrc)
        {
            Confidence::Certain
        } else {
            Confidence::Probable
        };
        drafts.push(Draft {
            confidence,
            identity: smallest(
                members
                    .iter()
                    .map(|&index| fingerprints[index].recording_identity(&tracks[index])),
            ),
            members,
        });
    }

    let mut groups = finish(tracks, drafts);
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

fn smallest(identities: impl Iterator<Item = String>) -> String {
    identities.min().unwrap_or_default()
}

/// Two groups can share an identity, say two pairs of `Intro`s too far apart
/// in duration to be one recording; their smallest track ID tells them apart.
fn finish(tracks: &[Track], drafts: Vec<Draft>) -> Vec<Group<'_>> {
    let mut seen: HashMap<(bool, &str), usize> = HashMap::new();
    for draft in &drafts {
        *seen
            .entry((draft.confidence == Confidence::Variant, &draft.identity))
            .or_default() += 1;
    }
    let shared: BTreeSet<(bool, String)> = seen
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|((variant, identity), _)| (variant, identity.to_owned()))
        .collect();
    drafts
        .into_iter()
        .map(|draft| {
            let variant = draft.confidence == Confidence::Variant;
            let mut identity = draft.identity;
            if shared.contains(&(variant, identity.clone())) {
                let first_id = draft
                    .members
                    .iter()
                    .map(|&index| tracks[index].id.as_str())
                    .min()
                    .unwrap_or_default();
                identity = format!("{identity}\u{1f}{first_id}");
            }
            group(
                tracks,
                draft.confidence,
                &group_key(variant, &identity),
                &draft.members,
            )
        })
        .collect()
}

fn group_key(variant: bool, identity: &str) -> String {
    let kind = if variant { "variant" } else { "duplicate" };
    let digest = Sha256::digest(format!("{kind}\0{identity}").as_bytes());
    digest[..12].iter().fold(String::new(), |mut key, byte| {
        let _ = write!(key, "{byte:02x}");
        key
    })
}

fn group<'a>(
    tracks: &'a [Track],
    confidence: Confidence,
    key: &str,
    members: &[usize],
) -> Group<'a> {
    let mut tracks: Vec<&Track> = members.iter().map(|&index| &tracks[index]).collect();
    tracks.sort_by(|a, b| {
        (&a.release_date, &a.added_at, &a.id).cmp(&(&b.release_date, &b.added_at, &b.id))
    });
    Group {
        key: key.to_owned(),
        confidence,
        tracks,
    }
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
    fn re_recording_with_new_guests_is_a_variant() {
        let tracks = [
            with_release(track("a", "Lola", &["Superbus"], 214), "2006-01-01"),
            with_release(
                track(
                    "b",
                    "Lola (feat. Nicola Sirkis)",
                    &["Superbus", "Hoshi", "Nicola Sirkis"],
                    216,
                ),
                "2025-03-14",
            ),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Variant, ids(&["a", "b"]))]
        );
    }

    #[test]
    fn guest_credited_in_the_artist_list_only_is_a_variant() {
        let tracks = [
            track("a", "Song", &["Artist"], 200),
            track("b", "Song", &["Artist", "Guest"], 201),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Variant, ids(&["a", "b"]))]
        );
    }

    #[test]
    fn guest_moved_from_the_title_to_the_artists_with_extra_copies() {
        let tracks = [
            track("a", "Song (feat. Guest)", &["Artist"], 200),
            track("b", "Song", &["Artist", "Guest"], 201),
            track("c", "Song (with Guest) - Remastered", &["Artist"], 199),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Probable, ids(&["a", "b", "c"]))]
        );
    }

    #[test]
    fn year_prefixed_remix_of_the_own_album_is_a_duplicate() {
        let tracks = [
            with_release(track("a", "Sheep", &["Pink Floyd"], 625), "1977-01-23"),
            with_release(
                track("b", "Sheep - 2018 Remix", &["Pink Floyd"], 624),
                "2018-09-28",
            ),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Probable, ids(&["a", "b"]))]
        );
    }

    #[test]
    fn remix_by_someone_else_stays_a_variant() {
        let tracks = [
            track("a", "Sheep", &["Pink Floyd"], 625),
            track("b", "Sheep - Some DJ Remix", &["Pink Floyd"], 624),
            track("c", "Sheep - Remix", &["Pink Floyd"], 623),
        ];
        assert_eq!(
            summary(&find_groups(&tracks, TOLERANCE)),
            [(Confidence::Variant, ids(&["a", "b", "c"]))]
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

    fn keys(groups: &[Group<'_>]) -> Vec<(Confidence, String)> {
        let mut keys: Vec<_> = groups
            .iter()
            .map(|group| (group.confidence, group.key.clone()))
            .collect();
        keys.sort();
        keys
    }

    #[test]
    fn group_keys_do_not_depend_on_the_library_order() {
        let tracks = vec![
            track("a", "Song", &["Artist"], 200),
            track("b", "Song - 2011 Remaster", &["Artist"], 201),
            track("c", "Song - Live", &["Artist"], 260),
            with_isrc(track("d", "Other", &["Artist"], 100), "USAAA0000001"),
            with_isrc(track("e", "Other", &["Artist"], 100), "USAAA0000001"),
        ];
        let mut reversed = tracks.clone();
        reversed.reverse();
        let groups = find_groups(&tracks, TOLERANCE);
        assert_eq!(groups.len(), 3);
        assert_eq!(keys(&groups), keys(&find_groups(&reversed, TOLERANCE)));
        let distinct: BTreeSet<&str> = groups.iter().map(|group| group.key.as_str()).collect();
        assert_eq!(distinct.len(), 3);
        assert!(
            groups
                .iter()
                .all(|group| group.key.len() == 24
                    && group.key.bytes().all(|b| b.is_ascii_hexdigit()))
        );
    }

    #[test]
    fn group_key_survives_a_new_copy_and_a_lost_isrc_match() {
        let pair = [
            with_isrc(track("a", "Song", &["Artist"], 200), "USAAA0000001"),
            with_isrc(track("b", "Song", &["Artist"], 200), "USAAA0000001"),
        ];
        let trio = [
            pair[0].clone(),
            pair[1].clone(),
            track("c", "Song (Deluxe)", &["Artist"], 201),
        ];
        let before = find_groups(&pair, TOLERANCE);
        let after = find_groups(&trio, TOLERANCE);
        assert_eq!(before[0].confidence, Confidence::Certain);
        assert_eq!(after[0].confidence, Confidence::Probable);
        assert_eq!(before[0].key, after[0].key);
    }

    #[test]
    fn homonym_groups_get_distinct_keys() {
        let tracks = [
            track("a", "Intro", &["Artist"], 60),
            track("b", "Intro", &["Artist"], 61),
            track("c", "Intro", &["Artist"], 95),
            track("d", "Intro", &["Artist"], 96),
        ];
        let groups = find_groups(&tracks, TOLERANCE);
        assert_eq!(groups.len(), 2);
        assert_ne!(groups[0].key, groups[1].key);
    }

    #[test]
    fn version_label_keeps_the_suffixes_as_written() {
        assert_eq!(
            version_label("Let It Be - Remastered 2009").as_deref(),
            Some("Remastered 2009")
        );
        assert_eq!(
            version_label("Song (Live) - 2011 Remaster").as_deref(),
            Some("Live · 2011 Remaster")
        );
        assert_eq!(version_label("Song (feat. Guest)"), None);
        assert_eq!(version_label("Song"), None);
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
