//! Check definitions, and the band a roll total lands in.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckDef {
    pub dice_count: u32,
    pub dice_sides: u32,
    pub stat: String,
}

/// One roll's band. A row without `partial_at` is `Success` or `Miss`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Partial,
    Miss,
}

/// `total` against `target`, with an optional lower band.
/// `total >= target` is success. `partial_at <= total < target` is partial.
/// Anything lower, and any binary roll, is a miss.
pub fn classify(total: i64, target: i64, partial_at: Option<i64>) -> Outcome {
    if total >= target {
        Outcome::Success
    } else if partial_at.is_some_and(|band| total >= band) {
        Outcome::Partial
    } else {
        Outcome::Miss
    }
}

#[cfg(test)]
mod tests {
    use super::{classify, Outcome};

    #[test]
    fn classification_meets_every_threshold() {
        let success = Outcome::Success;
        let partial = Outcome::Partial;
        let miss = Outcome::Miss;
        let cases = [
            (10, 10, None, success),
            (9, 10, None, miss),
            (11, 10, None, success),
            (10, 10, Some(7), success),
            (9, 10, Some(7), partial),
            (7, 10, Some(7), partial),
            (6, 10, Some(7), miss),
            (11, 10, Some(7), success),
            (0, 0, None, success),
            (0, 0, Some(-1), success),
            (-1, 0, Some(-1), partial),
            (-2, 0, Some(-1), miss),
        ];
        for (total, target, partial_at, expected) in cases {
            assert_eq!(
                classify(total, target, partial_at),
                expected,
                "{total} vs {target} band {partial_at:?}"
            );
        }
    }
}
