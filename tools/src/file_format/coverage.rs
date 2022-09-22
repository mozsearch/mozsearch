/// Interpolate coverage hits/misses for lines that didn't have coverage data,
/// as indicated by a -1.
///
/// Given coverage data where values are either -1 indicating no coverage or are
/// a coverage value >= 0, replace the -1 values with explicit interpolated hit
/// miss values:
/// * `-3`: Interpolated miss.
/// * `-2`: Interpolated hit.
///
/// The choice of using these additional values is because this might be
/// something that the upstream generator of the coverage data might do in the
/// future, and it already uses -1 as a magic value.
///
/// The goal of this interpolation is to minimize visual noise in the coverage
/// data.  Transitions to and from the uncovered (-1) state are informative but
/// are distracting and limit the ability to use preattentive processing
/// (see https://www.csc2.ncsu.edu/faculty/healey/PP/) to pick the more relevant
/// transitions between covered/uncovered.
///
/// It's straightforward to interpolate hits when there's an uncovered gap
/// between hits and likewise miss when there's an uncovered gap between misses.
/// The interesting questions are:
/// - What to do the start and end of the file.
/// - What to do when the uncovered gap is between a hit and a miss.  Extra
///   information about the AST or nesting contexts might help
///
/// Our arbitrary decisions here are:
/// - Leave the starts and ends of file uncovered.  This is more realistic but
///   at the cost of this realism making it less obvious that interpolation is
///   present in the rest of the file, potentially leading to bad inferences.
///   - We attempt to mitigate this by making sure the hover information makes
///     it clear when interpolation is at play so if someone looks into what's
///     going on they at least aren't misled for too long.
/// - Maximally interpolate hits over misses.  Our goal is that people's eyes
///   are drawn to misses.  This interpolation strategy makes sure that the
///   start and end of a run of misses are lines that are explicitly detected
///   as misses.
///
pub fn interpolate_coverage(mut raw: Vec<i64>) -> Vec<i64> {
    // We don't interpolate at the start or end of files, so start with already
    // having a valid -1 interpolation value.
    let mut have_interp_val = true;
    let mut interp_val = -1;
    // This value will never be used because we set have_interp_val to true
    // above which means we won't calculate an interpretation with this value.
    let mut last_noninterp_val = -1;
    for i in 0..raw.len() {
        let val = raw[i];
        // If we have a valid coverage value (=0 is miss, >0 is hit) then leave
        // the value as is, remember this value for interpolation and note that
        // we'll need to compute our next interpolation value.
        if val >= 0 {
            last_noninterp_val = val;
            have_interp_val = false;
            continue;
        }
        // Not a valid value, so we need to interpolate.

        // Did we already calculate our interpolation value?  If so, keep using
        // it.  (Note that at the start of the file we start our overwriting -1
        // with -1.)
        if have_interp_val {
            raw[i] = interp_val;
            continue;
        }

        // Check the next lines until we find a value that's >= 0.  If we don't
        // find any, then our end-of-file logic wants us to maintain a -1, so
        // configure for that base-case.
        have_interp_val = true;
        interp_val = -1;
        for j in (i + 1)..raw.len() {
            let next_val = raw[j];
            if next_val == -1 {
                continue;
            }
            // We've found a value which means that both last_noninterp_val and
            // next_val are >= 0.  (last_noninterp_val can never be -1 because
            // we start the loop with have_interp_val=true.)

            // Favor hits over misses (see func doc block for rationale).
            if next_val > 0 || last_noninterp_val > 0 {
                interp_val = -2;
            } else {
                interp_val = -3;
            }
            break;
        }
        raw[i] = interp_val;
    }
    raw
}

#[test]
fn test_interpolate_coverage() {
    let cases = vec![
        // empty
        vec![vec![], vec![]],
        // interpolate a hit between two hits
        vec![vec![1, -1, 1], vec![1, -2, 1]],
        // interpolate a miss between two misses
        vec![vec![0, -1, 0], vec![0, -3, 0]],
        // interpolate a hit if there's a hit on either side
        vec![vec![1, -1, 0, -1, 1], vec![1, -2, 0, -2, 1]],
        // don't interpolate ends
        vec![vec![-1, 1, -1, 1, -1], vec![-1, 1, -2, 1, -1]],
        // don't interpolate if the whole file is uncovered
        vec![vec![-1, -1, -1, -1, -1], vec![-1, -1, -1, -1, -1]],
        // combine all of the above (except for whole file), single interp each.
        vec![
            vec![-1, -1, 0, -1, 0, -1, 1, -1, 1, -1, 1, -1, 0, -1],
            vec![-1, -1, 0, -3, 0, -2, 1, -2, 1, -2, 1, -2, 0, -1],
        ],
        // now double the length of the interpolation runs
        vec![
            vec![
                -1, -1, 0, -1, -1, 0, -1, -1, 1, -1, -1, 1, -1, -1, 1, -1, -1, 0, -1,
            ],
            vec![
                -1, -1, 0, -3, -3, 0, -2, -2, 1, -2, -2, 1, -2, -2, 1, -2, -2, 0, -1,
            ],
        ],
        // now triple!
        vec![
            vec![
                -1, -1, 0, -1, -1, -1, 0, -1, -1, -1, 1, -1, -1, -1, 1, -1, -1, -1, 1, -1, -1, -1,
                0, -1,
            ],
            vec![
                -1, -1, 0, -3, -3, -3, 0, -2, -2, -2, 1, -2, -2, -2, 1, -2, -2, -2, 1, -2, -2, -2,
                0, -1,
            ],
        ],
        // add some runs of non-interpolated values to make sure we don't randomly clobber data.
        vec![
            vec![
                1, 2, 4, -1, 8, 16, 32, -1, -1, 64, 0, 0, -1, 0, 128, 256, -1, 512,
            ],
            vec![
                1, 2, 4, -2, 8, 16, 32, -2, -2, 64, 0, 0, -3, 0, 128, 256, -2, 512,
            ],
        ],
    ];

    for pair in cases {
        assert_eq!(interpolate_coverage(pair[0].clone()), pair[1]);
    }
}
