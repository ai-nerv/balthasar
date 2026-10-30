#![cfg(feature = "dense")]

mod support;

use candle_core::npy::NpzTensors;
use candle_core::pickle::{PthTensors, read_pth_tensor_info};
use candle_core::{Device, Error, Tensor};
use std::borrow::Cow;
use std::io::Write;
use std::path::Path;
use support::{Fixture, TestResult};

fn assert_npz(path: &Path, name: &str) -> TestResult {
    let tensors = Tensor::read_npz(path)?;
    assert_eq!(tensors.len(), 1);
    let (actual_name, tensor) = tensors.first().ok_or("missing tensor")?;
    assert_eq!(actual_name, name);
    assert_eq!(tensor.to_vec1::<u32>()?, [2, 4, 6]);
    let named = Tensor::read_npz_by_name(path, &[name])?;
    assert_eq!(
        named
            .first()
            .ok_or("missing named tensor")?
            .to_vec1::<u32>()?,
        [2, 4, 6]
    );
    let lazy = NpzTensors::new(path)?;
    assert_eq!(lazy.names(), vec![&name.to_owned()]);
    assert_eq!(
        lazy.get(name)?
            .ok_or("missing lazy tensor")?
            .to_vec1::<u32>()?,
        [2, 4, 6]
    );
    Ok(())
}

fn replace_name_byte(path: &Path, needle: &[u8], byte: u8) -> TestResult {
    let mut bytes = std::fs::read(path)?;
    let positions: Vec<_> = bytes
        .windows(needle.len())
        .enumerate()
        .filter_map(|(i, value)| (value == needle).then_some(i))
        .collect();
    assert_eq!(positions.len(), 2);
    for index in positions {
        *bytes.get_mut(index).ok_or("missing name")? = byte;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

#[test]
fn npz_utf8_and_cp437_names_load_eagerly_and_lazily() -> TestResult {
    let world = Fixture::new()?;
    let path = world.0.join("weights.npz");
    let tensor = Tensor::new(&[2u32, 4, 6], &Device::Cpu)?;
    Tensor::write_npz(&[("données", &tensor)], &path)?;
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path)?)?;
    assert!(matches!(
        zip.by_index(0)?.name()?,
        Cow::Borrowed("données.npy")
    ));
    assert_npz(&path, "données")?;
    Tensor::write_npz(&[("a", &tensor)], &path)?;
    replace_name_byte(&path, b"a.npy", 0x82)?;
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path)?)?;
    assert!(matches!(zip.by_index(0)?.name()?, Cow::Owned(name) if name == "é.npy"));
    assert_npz(&path, "é")
}

#[test]
fn non_utf8_names_use_the_zip_cp437_fallback() -> TestResult {
    let world = Fixture::new()?;
    let path = world.0.join("invalid.npz");
    let tensor = Tensor::new(&[2u32, 4, 6], &Device::Cpu)?;
    Tensor::write_npz(&[("é", &tensor)], &path)?;
    replace_name_byte(&path, "é.npy".as_bytes(), 0xff)?;
    assert_npz(&path, "\u{a0}⌐")?;
    Ok(())
}

#[test]
fn malformed_archives_are_rejected() -> TestResult {
    let world = Fixture::new()?;
    let path = world.0.join("invalid.zip");
    std::fs::write(&path, b"not a zip archive")?;
    assert!(matches!(Tensor::read_npz(&path), Err(Error::Zip(_))));
    assert!(matches!(NpzTensors::new(&path), Err(Error::Zip(_))));
    assert!(matches!(
        read_pth_tensor_info(&path, false, None),
        Err(Error::Zip(_))
    ));
    Ok(())
}

fn pth(path: &Path, prefix: &str) -> TestResult {
    let pickle = b"\x80\x02}X\x06\0\0\0weightctorch._utils\n_rebuild_tensor_v2\n((X\x07\0\0\0storagectorch\nFloatStorage\nX\x01\0\0\x000X\x03\0\0\0cpuK\x03tQK\0K\x03\x85K\x01\x85\x89ccollections\nOrderedDict\n)RtRs.";
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path)?);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file(format!("{prefix}/data.pkl"), options)?;
    zip.write_all(pickle)?;
    zip.start_file(format!("{prefix}/data/0"), options)?;
    for value in [2f32, 4., 6.] {
        zip.write_all(&value.to_le_bytes())?;
    }
    zip.finish()?;
    Ok(())
}

#[test]
fn pth_names_preserve_tensor_data() -> TestResult {
    let world = Fixture::new()?;
    let path = world.0.join("weights.pth");
    for prefix in ["archive", "données"] {
        pth(&path, prefix)?;
        let info = read_pth_tensor_info(&path, false, None)?;
        assert_eq!(info.len(), 1);
        let lazy = PthTensors::new(&path, None)?;
        assert_eq!(
            lazy.get("weight")?
                .ok_or("missing PTH tensor")?
                .to_vec1::<f32>()?,
            [2., 4., 6.]
        );
    }
    pth(&path, "a")?;
    replace_name_byte(&path, b"a/data.pkl", 0x82)?;
    replace_name_byte(&path, b"a/data/0", 0x82)?;
    let lazy = PthTensors::new(&path, None)?;
    assert_eq!(
        lazy.get("weight")?
            .ok_or("missing CP437 PTH tensor")?
            .to_vec1::<f32>()?,
        [2., 4., 6.]
    );
    Ok(())
}
