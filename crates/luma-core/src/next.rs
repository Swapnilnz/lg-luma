//! The episode after this one, from TMDB's season list.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Season {
    pub number: u32,
    pub episodes: u32,
}

/// The next episode: the next number in the season, else episode 1 of the next season with
/// episodes (specials, season 0, skipped). None after the series finale, or when the next one
/// is past `last_aired` (it hasn't aired yet).
pub fn after(
    (season, number): (u32, u32),
    seasons: &[Season],
    last_aired: Option<(u32, u32)>,
) -> Option<(u32, u32)> {
    let in_season = seasons
        .iter()
        .find(|s| s.number == season)
        .is_some_and(|s| number < s.episodes);
    let next = if in_season {
        Some((season, number + 1))
    } else {
        seasons
            .iter()
            .filter(|s| s.number > season && s.number > 0 && s.episodes > 0)
            .min_by_key(|s| s.number)
            .map(|s| (s.number, 1))
    }?;
    match last_aired {
        Some(aired) if next > aired => None,
        _ => Some(next),
    }
}

/// Where a show starts: S1E1, or the first regular season's first episode.
pub fn first(seasons: &[Season]) -> (u32, u32) {
    seasons
        .iter()
        .filter(|s| s.number > 0 && s.episodes > 0)
        .min_by_key(|s| s.number)
        .map_or((1, 1), |s| (s.number, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEASONS: [Season; 4] = [
        Season {
            number: 0,
            episodes: 14,
        },
        Season {
            number: 1,
            episodes: 10,
        },
        Season {
            number: 2,
            episodes: 10,
        },
        Season {
            number: 3,
            episodes: 0,
        },
    ];

    #[test]
    fn mid_season() {
        assert_eq!(after((1, 4), &SEASONS, None), Some((1, 5)));
    }

    #[test]
    fn season_finale_moves_to_the_next_season() {
        assert_eq!(after((1, 10), &SEASONS, None), Some((2, 1)));
    }

    #[test]
    fn series_finale_has_no_next() {
        // Season 3 has no episodes listed yet.
        assert_eq!(after((2, 10), &SEASONS, None), None);
    }

    #[test]
    fn specials_are_skipped_and_unaired_episodes_withheld() {
        assert_eq!(after((0, 3), &SEASONS, None), Some((0, 4)));
        assert_eq!(after((0, 14), &SEASONS, None), Some((1, 1)));
        assert_eq!(after((2, 4), &SEASONS, Some((2, 4))), None);
        assert_eq!(after((2, 4), &SEASONS, Some((2, 5))), Some((2, 5)));
        assert_eq!(first(&SEASONS), (1, 1));
        assert_eq!(first(&[]), (1, 1));
    }
}
