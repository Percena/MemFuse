use std::cmp::Reverse;
use std::collections::HashMap;

use mfs_index::SearchHit;

#[derive(Debug, Clone)]
pub struct RankedLayeredHit {
    pub hit: SearchHit,
    pub matched_levels: Vec<u8>,
    pub total_level_weight: u16,
}

pub fn rank_layered_hits(hits: Vec<SearchHit>, level_weights: [u16; 3]) -> Vec<SearchHit> {
    collapse_layered_hits(hits, level_weights)
        .into_iter()
        .map(|candidate| candidate.hit)
        .collect()
}

pub fn collapse_layered_hits(
    hits: Vec<SearchHit>,
    level_weights: [u16; 3],
) -> Vec<RankedLayeredHit> {
    let mut grouped: HashMap<String, Vec<SearchHit>> = HashMap::new();
    for hit in hits {
        grouped.entry(hit.uri.clone()).or_default().push(hit);
    }

    let mut collapsed = grouped
        .into_values()
        .map(|group| {
            let mut group = group;
            group.sort_by(|left, right| {
                layer_priority(left.level, level_weights)
                    .cmp(&layer_priority(right.level, level_weights))
                    .then_with(|| left.score.total_cmp(&right.score))
            });
            let representative = group.remove(0);
            let mut matched_levels = group
                .iter()
                .map(|hit| hit.level)
                .chain(std::iter::once(representative.level))
                .collect::<Vec<_>>();
            matched_levels.sort_unstable();
            matched_levels.dedup();
            let total_level_weight = matched_levels
                .iter()
                .map(|level| layer_priority(*level, level_weights))
                .sum();

            RankedLayeredHit {
                hit: representative,
                matched_levels,
                total_level_weight,
            }
        })
        .collect::<Vec<_>>();

    collapsed.sort_by(|left, right| {
        layer_priority(left.hit.level, level_weights)
            .cmp(&layer_priority(right.hit.level, level_weights))
            .then_with(|| {
                Reverse(left.matched_levels.len()).cmp(&Reverse(right.matched_levels.len()))
            })
            .then_with(|| left.total_level_weight.cmp(&right.total_level_weight))
            .then_with(|| left.hit.score.total_cmp(&right.hit.score))
            .then_with(|| left.hit.uri.cmp(&right.hit.uri))
    });

    collapsed
}

fn layer_priority(level: u8, level_weights: [u16; 3]) -> u16 {
    match level {
        0 => level_weights[0],
        1 => level_weights[1],
        _ => level_weights[2],
    }
}

#[cfg(test)]
mod tests {
    use mfs_index::SearchHit;

    use super::rank_layered_hits;

    #[test]
    fn rank_layered_hits_prefers_multi_level_support_with_same_best_layer() {
        let ranked = rank_layered_hits(
            vec![
                SearchHit {
                    uri: "mfs://resources/localfs/docs/guides".to_owned(),
                    context_type: "resource".to_owned(),
                    level: 0,
                    score: -0.8,
                    excerpt: "guides abstract".to_owned(),
                },
                SearchHit {
                    uri: "mfs://resources/localfs/docs/guides".to_owned(),
                    context_type: "resource".to_owned(),
                    level: 1,
                    score: -0.7,
                    excerpt: "guides overview".to_owned(),
                },
                SearchHit {
                    uri: "mfs://resources/localfs/docs".to_owned(),
                    context_type: "resource".to_owned(),
                    level: 0,
                    score: -0.9,
                    excerpt: "docs abstract".to_owned(),
                },
            ],
            [0, 100, 200],
        );

        assert_eq!(ranked[0].uri, "mfs://resources/localfs/docs/guides");
    }
}
