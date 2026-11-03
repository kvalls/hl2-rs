use super::*;

#[test]
fn a_lookup_file_becomes_a_32_cubed_rgba_volume_in_file_order() {
    let file: Vec<u8> = (0..FILE_BYTES).map(|i| (i % 251) as u8).collect();
    let image = lookup_image(&file).expect("98304 bytes");
    assert_eq!(
        (
            image.width(),
            image.height(),
            image.texture_descriptor.size.depth_or_array_layers
        ),
        (32, 32, 32)
    );
    assert_eq!(image.texture_descriptor.dimension, TextureDimension::D3);
    let data = image.data.as_ref().expect("data");
    assert_eq!(data.len(), 32 * 32 * 32 * 4);
    // Texel n holds the file's bytes 3n..3n+3 as R, G, B; x runs fastest, then y, then z.
    assert_eq!(&data[4..8], &[file[3], file[4], file[5], 255]);
    let n = 1024 + 32 + 1;
    assert_eq!(
        &data[n * 4..n * 4 + 4],
        &[file[n * 3], file[n * 3 + 1], file[n * 3 + 2], 255]
    );
}

#[test]
fn a_file_of_any_other_size_is_refused() {
    assert!(lookup_image(&[0; FILE_BYTES - 1]).is_none());
    assert!(lookup_image(&[0; FILE_BYTES + 3]).is_none());
    assert!(lookup_image(&[]).is_none());
}

#[test]
fn files_are_read_again_only_when_the_list_changes() {
    let mut lookups = Lookups::default();
    let files = vec!["scripts/a.raw".to_string()];
    lookups.load(&files, None);
    assert!(
        matches!(lookups.state, State::Failed),
        "no content to read from"
    );
    lookups.state = State::Idle;
    lookups.load(&files, None);
    assert!(
        matches!(lookups.state, State::Idle),
        "the same list is not read again"
    );
    lookups.load(&["scripts/b.raw".to_string()], None);
    assert!(matches!(lookups.state, State::Failed));
}

#[test]
fn finished_reads_become_images_and_a_failure_is_kept() {
    let mut images = Assets::<Image>::default();
    let mut lookups = Lookups {
        state: State::Loading(Arc::new(Mutex::new(Some(Ok(vec![vec![0; FILE_BYTES]; 2]))))),
        ..Default::default()
    };
    lookups.finish(&mut images);
    let State::Ready(volumes) = &lookups.state else {
        panic!("ready");
    };
    assert_eq!(volumes.len(), 2);
    assert_eq!(images.len(), 2);
    lookups.state = State::Loading(Arc::new(Mutex::new(Some(Err("missing".into())))));
    lookups.finish(&mut images);
    assert!(matches!(lookups.state, State::Failed));
    lookups.state = State::Loading(Arc::default());
    lookups.finish(&mut images);
    assert!(matches!(lookups.state, State::Loading(_)), "still reading");
}
