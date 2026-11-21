//! Displacement cells split along alternating diagonals, as the engine and the compilers do.
use super::disp_cell_triangles;

#[test]
fn cell_diagonals_alternate_like_a_checkerboard() {
    // A 3x3-vertex grid: cells start at vertices 0, 1, 3 and 4.
    assert_eq!(disp_cell_triangles(0, 3), [0, 3, 4, 0, 4, 1]);
    assert_eq!(disp_cell_triangles(1, 3), [1, 4, 2, 2, 4, 5]);
    assert_eq!(disp_cell_triangles(3, 3), [3, 6, 4, 4, 6, 7]);
    assert_eq!(disp_cell_triangles(4, 3), [4, 7, 8, 4, 8, 5]);
}

#[test]
fn both_splits_keep_the_winding_of_the_cell() {
    // Vertices on a flat grid (x = column, y = row): every triangle turns the same way.
    let at = |i: u32| ((i % 3) as f32, (i / 3) as f32);
    for index in [0, 1, 3, 4] {
        for t in disp_cell_triangles(index, 3).chunks(3) {
            let [(ax, ay), (bx, by), (cx, cy)] = [at(t[0]), at(t[1]), at(t[2])];
            let turn = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
            assert!(turn < 0., "cell {index}: {t:?}");
        }
    }
}

#[test]
fn dispvert_alpha_becomes_a_clamped_byte() {
    use super::disp_alpha;
    assert_eq!(disp_alpha(0.), 0);
    assert_eq!(disp_alpha(127.6), 128);
    assert_eq!(disp_alpha(255.), 255);
    assert_eq!(disp_alpha(300.), 255);
    assert_eq!(disp_alpha(-4.), 0);
    assert_eq!(disp_alpha(f32::NAN), 0);
}
