//! The word-list variants, the way the website offers them.
//!
//! The website has no size-switching logic at all: `english_1k` and `english_5k`
//! are separate published files with different ids, and the language dropdown
//! lists them as ordinary entries. A client that only cycled through six base
//! languages was therefore offering a fraction of what the site does — the whole
//! point of a bigger list is that it is *less* repetitive, and `english_5k` is
//! the difference between a test that feels fresh and one that has used "the" and
//! "of" forty times.
//!
//! Rather than hard-code four hundred ids, the variants of a base language are
//! derived: everything up to the size suffix is the same test with a longer list.
//! Upstream ids are `<base>` and `<base>_<n>k`, so `english` yields
//! [`VARIANTS`] plus a size for each, and the whole thing is checked against the
//! embedded lists in the tests below.

/// The list sizes the website offers for the languages it ships most of, largest
/// last.
///
/// These are the sizes upstream actually publishes for the popular base
/// languages. A language with only a base list — which is most of the four
/// hundred — still works; [`Language::variants`] just yields the base.
pub const SIZES: [u32; 5] = [0, 1, 5, 10, 25];

/// The largest list upstream ships for English, in words.
pub const LARGEST_ENGLISH: u32 = 450;

/// Base languages offered in the settings browser.
///
/// Upstream publishes about four hundred, and listing all of them would mean a
/// picker nobody scrolls to the end of and a config file nobody edits by hand.
/// These are the ones a test is actually run in, in the order they are worth
/// offering, and the base lists for the first six of them are in the binary — so
/// picking any of these works offline and picking any of the rest costs one
/// download.
///
/// The sizes are not listed here: they are derived from the base, which is the
/// whole point of the scheme.
pub const POPULAR_BASES: &[&str] = &[
    "english",
    "russian",
    "german",
    "spanish",
    "french",
    "portuguese",
    "italian",
    "polish",
    "dutch",
    "ukrainian",
    "romanian",
    "czech",
    "swedish",
    "turkish",
    "norwegian",
    "finnish",
    "danish",
    "greek",
    "hungarian",
    "bulgarian",
    "serbian",
    "slovak",
    "slovenian",
    "croatian",
    "hebrew",
    "arabic",
    "hindi",
    "chinese_simplified",
    "japanese",
    "korean",
    "vietnamese",
    "thai",
    "indonesian",
    "estonian",
    "latvian",
    "lithuanian",
    "kazakh",
    "swiss_german",
    "portuguese_brazilian",
    "tagalog",
];

/// A word list the user can pick, and the file it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Language {
    /// The upstream id, e.g. `english_5k`. This is the cache file's name and what
    /// a result payload reports.
    pub id: String,
    /// The base language, e.g. `english`. Quotes are chosen per base language, so
    /// the size suffix has to be strippable.
    pub base: String,
    /// How many words the list has, or `0` when the list is not embedded and the
    /// count is not known without fetching it.
    pub words: u32,
    /// Whether the list is already in the binary.
    pub embedded: bool,
}

impl Language {
    /// What the bar shows: the base name, with the size only when there is one.
    ///
    /// A terminal bar is short, so `english 1k` rather than `english_1k`, and
    /// nothing at all for the base list — the reader knows that is english.
    pub fn label(&self) -> String {
        match self.size() {
            Some(0) | None => self.base.clone(),
            Some(size) => format!("{} {}k", self.base, size),
        }
    }

    /// The list size in thousands, where the id carries one.
    pub fn size(&self) -> Option<u32> {
        split_id(&self.id).1
    }

    /// Whether this is the plain base list rather than a sized variant.
    pub fn is_base(&self) -> bool {
        split_id(&self.id).1.is_none()
    }
}

/// Splits an id into its base and its size suffix in thousands.
///
/// `english` → `("english", None)`; `english_5k` → `("english", Some(5))`.
/// A suffix that is not a size — `english_1k` is, `english_doubleletter` is not
/// — is left on the base, so a dialect or a special list keeps its whole name.
pub fn split_id(id: &str) -> (&str, Option<u32>) {
    let Some((base, suffix)) = id.rsplit_once('_') else {
        return (id, None);
    };
    let Some(digits) = suffix.strip_suffix('k') else {
        return (id, None);
    };
    match digits.parse::<u32>() {
        Ok(size) => (base, Some(size)),
        Err(_) => (id, None),
    }
}

/// The base name a size suffix should be stripped from when looking a language's
/// quotes up.
///
/// The website's `removeLanguageSize` does exactly this with one regex, and the
/// reason it exists is that quote files are keyed by base language only: there
/// is no `quotes/english_5k.json`.
pub fn base_of(id: &str) -> &str {
    split_id(id).0
}

/// Every size variant of `base`, base first.
///
/// The embedded variants are known to exist; the rest are offered optimistically
/// and fall back if a fetch finds nothing, because upstream does not publish a
/// 25k list for most languages and a test that quietly falls back is better than a
/// dropdown that hides an option per language.
pub fn variants(base: &str) -> Vec<String> {
    let mut ids = vec![base.to_owned()];
    for size in SIZES {
        if size == 0 {
            continue;
        }
        ids.push(format!("{base}_{size}k"));
    }
    ids
}

/// The ids to offer for a base language, filtered to the ones actually available.
///
/// Availability is: embedded, or already in the on-disk cache. The rest are not
/// offered, because a language picker full of entries that fail to download is
/// worse than one that grows as the cache does.
pub fn available(base: &str, cached: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut ids = vec![base.to_owned()];
    for size in SIZES {
        if size == 0 {
            continue;
        }
        let id = format!("{base}_{size}k");
        if embedded(&id) || cached(&id) {
            ids.push(id);
        }
    }
    ids
}

/// Whether an id is one of the lists compiled into the binary.
pub fn embedded(id: &str) -> bool {
    crate::words::language::embedded(id).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_id_has_no_size() {
        assert_eq!(split_id("english"), ("english", None));
        assert_eq!(split_id("english").1, None);
    }

    #[test]
    fn a_sized_id_splits_into_a_base_and_a_size() {
        assert_eq!(split_id("english_5k"), ("english", Some(5)));
        assert_eq!(split_id("english_10k"), ("english", Some(10)));
        assert_eq!(split_id("english_25k"), ("english", Some(25)));
        assert_eq!(split_id("russian_1k"), ("russian", Some(1)));
    }

    /// Upstream ids that merely contain an underscore are not sizes, and reading
    /// them as one would strip the suffix off a dialect.
    #[test]
    fn an_underscore_that_is_not_a_size_stays_on_the_base() {
        for id in [
            "english_doubleletter",
            "english_contractions",
            "english_commonly_misspelled",
            "english_medical",
            "swiss_german",
            "portuguese_brazilian",
            "russian_kazakh",
        ] {
            assert_eq!(split_id(id), (id, None), "{id} was read as a size");
        }
    }

    /// Quote files are keyed by base language only, which is why the size has to
    /// come off before looking one up.
    #[test]
    fn a_quote_lookup_strips_the_size() {
        assert_eq!(base_of("english_5k"), "english");
        assert_eq!(base_of("english"), "english");
        assert_eq!(base_of("english_doubleletter"), "english_doubleletter");
    }

    #[test]
    fn the_variants_are_the_base_and_four_sizes() {
        let ids = variants("english");
        assert_eq!(
            ids,
            [
                "english",
                "english_1k",
                "english_5k",
                "english_10k",
                "english_25k"
            ]
        );
    }

    #[test]
    fn every_variant_shares_one_base() {
        for id in variants("german") {
            assert_eq!(base_of(&id), "german", "{id}");
        }
    }

    /// A picker that only offers what is there beats one full of entries that
    /// fail to download.
    #[test]
    fn availability_is_embedded_or_cached() {
        let nothing = |_: &str| false;
        assert_eq!(
            available("english", &nothing),
            ["english"],
            "with nothing embedded beyond the base, only the base is offered"
        );

        let has_5k = |id: &str| id == "english_5k";
        assert_eq!(
            available("english", &has_5k),
            ["english", "english_5k"],
            "a cached size appears"
        );
    }

    #[test]
    fn the_embedded_base_lists_are_offered() {
        assert!(embedded("english"));
        assert!(embedded("russian"));
        assert!(!embedded("klingon"));
    }

    #[test]
    fn a_label_shows_the_size_only_when_there_is_one() {
        let sized = Language {
            id: "english_5k".to_owned(),
            base: "english".to_owned(),
            words: 5000,
            embedded: false,
        };
        assert_eq!(sized.label(), "english 5k");
        assert!(!sized.is_base());

        let base = Language {
            id: "english".to_owned(),
            base: "english".to_owned(),
            words: 200,
            embedded: true,
        };
        assert_eq!(base.label(), "english");
        assert!(base.is_base());
    }

    /// The browser has to offer something, and it has to offer the base lists
    /// that are in the binary first — picking one of those must work with no
    /// network at all.
    #[test]
    fn the_offered_bases_start_with_the_embedded_ones() {
        let embedded: Vec<&str> = crate::words::language::embedded_ids().collect();
        for id in &embedded {
            assert!(
                POPULAR_BASES.contains(id),
                "{id} is embedded but not offered in the browser"
            );
        }
        // And the first entry is the one most tests are run in.
        assert_eq!(POPULAR_BASES[0], "english");
    }

    #[test]
    fn the_offered_bases_have_no_sizes_in_them() {
        // A base with a size suffix in the list would produce `english_5k_1k`.
        for base in POPULAR_BASES {
            assert_eq!(split_id(base).1, None, "{base} carries a size");
        }
    }

    #[test]
    fn the_offered_bases_have_no_duplicates() {
        let mut seen = std::collections::BTreeSet::new();
        for base in POPULAR_BASES {
            assert!(seen.insert(*base), "{base} is listed twice");
        }
    }

    /// Every offered base must produce a valid id for every size.
    #[test]
    fn every_offered_base_has_valid_variants() {
        for base in POPULAR_BASES {
            for id in variants(base) {
                assert!(!id.contains("__"), "{id} is malformed");
                assert_eq!(base_of(&id), *base, "{id}");
            }
        }
    }

    #[test]
    fn a_dialect_keeps_its_whole_name_as_a_label() {
        let dialect = Language {
            id: "english_doubleletter".to_owned(),
            base: "english_doubleletter".to_owned(),
            words: 0,
            embedded: false,
        };
        assert_eq!(dialect.label(), "english_doubleletter");
    }
}
