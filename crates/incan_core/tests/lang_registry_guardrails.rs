use std::collections::HashMap;

use incan_core::lang::builtins;
use incan_core::lang::keywords;
use incan_core::lang::operators;
use incan_core::lang::punctuation;
use incan_core::lang::types::{collections, numerics, stringlike};

#[test]
fn keywords_spellings_unique_and_resolvable() {
    let mut seen: HashMap<&'static str, keywords::KeywordId> = HashMap::new();

    for info in keywords::KEYWORDS {
        assert_eq!(
            keywords::from_str(info.canonical),
            Some(info.id),
            "keyword canonical spelling not resolvable: {}",
            info.canonical
        );
        assert_eq!(
            keywords::as_str(info.id),
            info.canonical,
            "keyword as_str mismatch for {:?}",
            info.id
        );

        if let Some(prev) = seen.insert(info.canonical, info.id) {
            panic!(
                "duplicate keyword spelling {:?}: {:?} and {:?}",
                info.canonical, prev, info.id
            );
        }

        for &alias in info.aliases {
            assert_eq!(
                keywords::from_str(alias),
                Some(info.id),
                "keyword alias not resolvable: {}",
                alias
            );
            if let Some(prev) = seen.insert(alias, info.id) {
                panic!(
                    "duplicate keyword alias spelling {:?}: {:?} and {:?}",
                    alias, prev, info.id
                );
            }
        }
    }
}

#[test]
fn builtins_spellings_unique_and_resolvable() {
    let mut seen: HashMap<&'static str, builtins::BuiltinFnId> = HashMap::new();

    for info in builtins::BUILTIN_FUNCTIONS {
        assert_eq!(
            builtins::from_str(info.canonical),
            Some(info.id),
            "builtin canonical spelling not resolvable: {}",
            info.canonical
        );
        assert_eq!(
            builtins::as_str(info.id),
            info.canonical,
            "builtin as_str mismatch for {:?}",
            info.id
        );

        if let Some(prev) = seen.insert(info.canonical, info.id) {
            panic!(
                "duplicate builtin spelling {:?}: {:?} and {:?}",
                info.canonical, prev, info.id
            );
        }

        for &alias in info.aliases {
            assert_eq!(
                builtins::from_str(alias),
                Some(info.id),
                "builtin alias not resolvable: {}",
                alias
            );
            if let Some(prev) = seen.insert(alias, info.id) {
                panic!(
                    "duplicate builtin alias spelling {:?}: {:?} and {:?}",
                    alias, prev, info.id
                );
            }
        }
    }
}

#[test]
fn operators_spellings_unique_and_resolvable() {
    let mut seen: HashMap<&'static str, operators::OperatorId> = HashMap::new();

    for info in operators::OPERATORS {
        for &sp in info.spellings {
            assert_eq!(
                operators::from_str(sp),
                Some(info.id),
                "operator spelling not resolvable: {}",
                sp
            );
            if let Some(prev) = seen.insert(sp, info.id) {
                panic!("duplicate operator spelling {:?}: {:?} and {:?}", sp, prev, info.id);
            }
        }
    }
}

#[test]
fn punctuation_spellings_unique_and_resolvable() {
    let mut seen: HashMap<&'static str, punctuation::PunctuationId> = HashMap::new();

    for info in punctuation::PUNCTUATION {
        assert_eq!(
            punctuation::from_str(info.canonical),
            Some(info.id),
            "punctuation canonical spelling not resolvable: {}",
            info.canonical
        );
        assert_eq!(
            punctuation::as_str(info.id),
            info.canonical,
            "punctuation as_str mismatch for {:?}",
            info.id
        );

        if let Some(prev) = seen.insert(info.canonical, info.id) {
            panic!(
                "duplicate punctuation spelling {:?}: {:?} and {:?}",
                info.canonical, prev, info.id
            );
        }

        for &alias in info.aliases {
            assert_eq!(
                punctuation::from_str(alias),
                Some(info.id),
                "punctuation alias not resolvable: {}",
                alias
            );
            if let Some(prev) = seen.insert(alias, info.id) {
                panic!(
                    "duplicate punctuation alias spelling {:?}: {:?} and {:?}",
                    alias, prev, info.id
                );
            }
        }
    }
}

#[test]
fn types_spellings_unique_and_resolvable() {
    // stringlike
    {
        let mut seen: HashMap<&'static str, stringlike::StringLikeId> = HashMap::new();
        for info in stringlike::STRING_LIKE_TYPES {
            if let Some(prev) = seen.insert(info.canonical, info.id) {
                panic!(
                    "duplicate stringlike canonical {:?}: {:?} and {:?}",
                    info.canonical, prev, info.id
                );
            }
            for &alias in info.aliases {
                if let Some(prev) = seen.insert(alias, info.id) {
                    panic!("duplicate stringlike alias {:?}: {:?} and {:?}", alias, prev, info.id);
                }
            }
            assert_eq!(
                stringlike::as_str(info.id),
                info.canonical,
                "stringlike as_str mismatch for {:?}",
                info.id
            );
        }
    }

    // numerics
    {
        let mut seen: HashMap<&'static str, numerics::NumericTypeId> = HashMap::new();
        for info in numerics::NUMERIC_TYPES {
            if let Some(prev) = seen.insert(info.canonical, info.id) {
                panic!(
                    "duplicate numeric canonical {:?}: {:?} and {:?}",
                    info.canonical, prev, info.id
                );
            }
            for &alias in info.aliases {
                if let Some(prev) = seen.insert(alias, info.id) {
                    panic!("duplicate numeric alias {:?}: {:?} and {:?}", alias, prev, info.id);
                }
            }
            assert_eq!(
                numerics::as_str(info.id),
                info.canonical,
                "numeric as_str mismatch for {:?}",
                info.id
            );
        }
    }

    // collections
    {
        let mut seen: HashMap<&'static str, collections::CollectionTypeId> = HashMap::new();
        for info in collections::COLLECTION_TYPES {
            if let Some(prev) = seen.insert(info.canonical, info.id) {
                panic!(
                    "duplicate collection canonical {:?}: {:?} and {:?}",
                    info.canonical, prev, info.id
                );
            }
            for &alias in info.aliases {
                if let Some(prev) = seen.insert(alias, info.id) {
                    panic!("duplicate collection alias {:?}: {:?} and {:?}", alias, prev, info.id);
                }
            }
            assert_eq!(
                collections::as_str(info.id),
                info.canonical,
                "collection as_str mismatch for {:?}",
                info.id
            );
        }
    }
}
