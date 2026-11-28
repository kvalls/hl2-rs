use super::*;

/// Four vertices, two triangles sharing an edge, plus a third triangle; vertex 3 is the only
/// one a flex moves.
fn face() -> Batch {
    let positions = vec![
        [0., 0., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
        [1., 1., 0.],
        [2., 0., 0.],
    ];
    let n = positions.len();
    Batch {
        decal_layer: None,
        uvs: (0..n).map(|i| [i as f32, 0.]).collect(),
        light_uvs: vec![[0.; 2]; n],
        colors: (0..n).map(|i| [i as f32; 4]).collect(),
        normals: vec![glam::Vec3::Z; n],
        model: true,
        indices: vec![0, 1, 2, 2, 1, 3, 1, 4, 2],
        skin: positions
            .iter()
            .map(|p| (glam::Vec3::from_array(*p), None))
            .collect(),
        flex: (0..n).map(|i| Some([0, 0, i as u16])).collect(),
        positions,
    }
}
/// Each triangle's corner positions, for comparing geometry across splits.
fn triangles(batch: &Batch) -> Vec<[[f32; 3]; 3]> {
    let corners = batch.indices.as_chunks::<3>().0.iter();
    corners
        .map(|t| t.map(|i| batch.positions[i as usize]))
        .collect()
}

#[test]
fn a_face_splits_into_rigid_and_moving_triangles_with_their_own_vertices() {
    let movable = HashSet::from([[0, 0, 3]]);
    let parts = face().split_flexed(&movable);
    assert_eq!(parts.len(), 2);
    let (rigid, moving) = (&parts[0], &parts[1]);
    assert!(rigid.consistent() && moving.consistent());
    // The rigid draw has the triangles without vertex 3, the moving one the triangle with it.
    let all = triangles(&face());
    assert_eq!(triangles(rigid), vec![all[0], all[2]]);
    assert_eq!(triangles(moving), vec![all[1]]);
    assert_eq!(rigid.positions.len(), 4);
    assert_eq!(moving.positions.len(), 3);
    // Every attribute and flex reference travels with its vertex.
    for part in &parts {
        assert!(part.model && part.flex.len() == part.positions.len());
        for (i, position) in part.positions.iter().enumerate() {
            let source = face().positions.iter().position(|p| p == position).unwrap();
            assert_eq!(part.uvs[i], [source as f32, 0.]);
            assert_eq!(part.colors[i], [source as f32; 4]);
            assert_eq!(part.flex[i], Some([0, 0, source as u16]));
            assert_eq!(part.skin[i].0, glam::Vec3::from_array(*position));
        }
    }
}

#[test]
fn faces_with_nothing_or_everything_movable_stay_whole() {
    assert_eq!(face().split_flexed(&HashSet::new()).len(), 1);
    let everything = (0..5).map(|i| [0, 0, i]).collect();
    assert_eq!(face().split_flexed(&everything).len(), 1);
    let mut broken = face();
    broken.uvs.pop();
    assert_eq!(broken.split_flexed(&HashSet::from([[0, 0, 3]])).len(), 1);
}
