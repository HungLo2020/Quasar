//! CPU-side checks for the offscreen `Image` type.
use quasar_frame::Image;

#[test]
fn images_validate_length_and_index_pixels() {
    assert!(Image::new(2, 2, vec![0; 15]).is_err());
    let mut rgba = vec![0u8; 2 * 2 * 4];
    rgba[12..16].copy_from_slice(&[1, 2, 3, 4]);
    let image = Image::new(2, 2, rgba).unwrap();
    assert_eq!(image.pixel(1, 1), Some([1, 2, 3, 4]));
    assert_eq!(image.pixel(0, 0), Some([0, 0, 0, 0]));
    assert_eq!(image.pixel(2, 0), None);
}

#[cfg(feature = "png")]
#[test]
fn images_encode_and_write_decodable_pngs() {
    let rgba: Vec<u8> = (0..3 * 2 * 4).map(|i| (i * 10) as u8).collect();
    let image = Image::new(3, 2, rgba.clone()).unwrap();
    let bytes = image.encode_png().unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).unwrap();
    assert_eq!((info.width, info.height), (3, 2));
    assert_eq!(&buffer[..info.buffer_size()], &rgba[..]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frame.png");
    image.write_png(&path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(Image::new(0, 0, Vec::new()).unwrap().encode_png().is_err());
}
