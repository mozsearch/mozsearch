use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy, Debug)]
pub enum InterpolatedCoverage {
    Covered(u32),
    Uncovered,
    InterpolatedHit,
    InterpolatedMiss,
}

/// Interpolate coverage hits/misses for lines that didn't have coverage data.
///
/// Given coverage data where values are either None indicating no coverage or
/// are a coverage value >= 0, replace the None values with explicit
/// interpolated hit/miss values except at the start and end of files.
///
/// The goal of this interpolation is to minimize visual noise in the coverage
/// data.  Transitions to and from the uncovered (None) state are informative
/// but are distracting and limit the ability to use preattentive processing
/// (see https://www.csc2.ncsu.edu/faculty/healey/PP/) to pick the more relevant
/// transitions between covered/uncovered.
pub fn interpolate_coverage(
    mut raw: impl Iterator<Item = Option<u32>> + Clone,
) -> Vec<InterpolatedCoverage> {
    use InterpolatedCoverage::*;

    let (min_size, max_size) = raw.size_hint();
    let mut interpolated = Vec::with_capacity(max_size.unwrap_or(min_size));

    // We don't interpolate at the start or end of files, so start with already
    // having a valid Uncovered interpolation value.
    let mut last_interp_val = Some(Uncovered);
    let mut last_noninterp_val = None;

    while let Some(val) = raw.next() {
        // If we have a valid coverage value then leave the value as is,
        // remember this value for interpolation and note that we'll need to
        // compute our next interpolation value.
        if let Some(val) = val {
            last_noninterp_val = Some(val);
            last_interp_val = None;
            interpolated.push(Covered(val));
            continue;
        }
        // Not a valid value, so we need to interpolate.

        // Did we already calculate our interpolation value?  If so, keep using
        // it.  (Note that at the start of the file we start with Uncovered.)
        if let Some(interp_val) = last_interp_val {
            interpolated.push(interp_val);
            continue;
        }

        // Find the next non-None value by forking the iterator at the current
        // location. Iterator<Item=Option>::flatten skips over None values.
        let next_val = raw.clone().flatten().next();
        let interp_val = match (last_noninterp_val, next_val) {
            (Some(0), Some(0)) => InterpolatedMiss,
            (Some(_), Some(_)) => InterpolatedHit,
            (_, _) => Uncovered,
        };
        last_interp_val = Some(interp_val);

        interpolated.push(interp_val);
    }

    interpolated
}

#[test]
fn test_interpolate_coverage() {
    use InterpolatedCoverage::*;

    let cases: [(&[_], &[_]); _] = [
        // empty
        (&[], &[]),
        // interpolate a hit between two hits
        (
            &[Some(1), None, Some(1)],
            &[Covered(1), InterpolatedHit, Covered(1)],
        ),
        // interpolate a miss between two misses
        (
            &[Some(0), None, Some(0)],
            &[Covered(0), InterpolatedMiss, Covered(0)],
        ),
        // interpolate a hit if there's a hit on either side
        (
            &[Some(1), None, Some(0), None, Some(1)],
            &[
                Covered(1),
                InterpolatedHit,
                Covered(0),
                InterpolatedHit,
                Covered(1),
            ],
        ),
        // don't interpolate ends
        (
            &[None, Some(1), None, Some(1), None],
            &[
                Uncovered,
                Covered(1),
                InterpolatedHit,
                Covered(1),
                Uncovered,
            ],
        ),
        // don't interpolate if the whole file is uncovered
        (&[None; 5], &[Uncovered; 5]),
        // combine all of the above (except for whole file), single interp each.
        (
            &[
                None,
                None,
                Some(0),
                None,
                Some(0),
                None,
                Some(1),
                None,
                Some(1),
                None,
                Some(1),
                None,
                Some(0),
                None,
            ],
            &[
                Uncovered,
                Uncovered,
                Covered(0),
                InterpolatedMiss,
                Covered(0),
                InterpolatedHit,
                Covered(1),
                InterpolatedHit,
                Covered(1),
                InterpolatedHit,
                Covered(1),
                InterpolatedHit,
                Covered(0),
                Uncovered,
            ],
        ),
        // now double the length of the interpolation runs
        (
            &[
                None,
                None,
                Some(0),
                None,
                None,
                Some(0),
                None,
                None,
                Some(1),
                None,
                None,
                Some(1),
                None,
                None,
                Some(1),
                None,
                None,
                Some(0),
                None,
            ],
            &[
                Uncovered,
                Uncovered,
                Covered(0),
                InterpolatedMiss,
                InterpolatedMiss,
                Covered(0),
                InterpolatedHit,
                InterpolatedHit,
                Covered(1),
                InterpolatedHit,
                InterpolatedHit,
                Covered(1),
                InterpolatedHit,
                InterpolatedHit,
                Covered(1),
                InterpolatedHit,
                InterpolatedHit,
                Covered(0),
                Uncovered,
            ],
        ),
        // now triple!
        (
            &[
                None,
                None,
                Some(0),
                None,
                None,
                None,
                Some(0),
                None,
                None,
                None,
                Some(1),
                None,
                None,
                None,
                Some(1),
                None,
                None,
                None,
                Some(1),
                None,
                None,
                None,
                Some(0),
                None,
            ],
            &[
                Uncovered,
                Uncovered,
                Covered(0),
                InterpolatedMiss,
                InterpolatedMiss,
                InterpolatedMiss,
                Covered(0),
                InterpolatedHit,
                InterpolatedHit,
                InterpolatedHit,
                Covered(1),
                InterpolatedHit,
                InterpolatedHit,
                InterpolatedHit,
                Covered(1),
                InterpolatedHit,
                InterpolatedHit,
                InterpolatedHit,
                Covered(1),
                InterpolatedHit,
                InterpolatedHit,
                InterpolatedHit,
                Covered(0),
                Uncovered,
            ],
        ),
        // add some runs of non-interpolated values to make sure we don't randomly clobber data.
        (
            &[
                Some(1),
                Some(2),
                Some(4),
                None,
                Some(8),
                Some(16),
                Some(32),
                None,
                None,
                Some(64),
                Some(0),
                Some(0),
                None,
                Some(0),
                Some(128),
                Some(256),
                None,
                Some(512),
            ],
            &[
                Covered(1),
                Covered(2),
                Covered(4),
                InterpolatedHit,
                Covered(8),
                Covered(16),
                Covered(32),
                InterpolatedHit,
                InterpolatedHit,
                Covered(64),
                Covered(0),
                Covered(0),
                InterpolatedMiss,
                Covered(0),
                Covered(128),
                Covered(256),
                InterpolatedHit,
                Covered(512),
            ],
        ),
    ];

    for (input, expected_output) in cases {
        assert_eq!(interpolate_coverage(input.iter().copied()), expected_output);
    }
}
