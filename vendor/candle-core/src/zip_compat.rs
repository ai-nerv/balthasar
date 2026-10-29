use std::io::{Read, Seek};
use zip::result::{ZipError, ZipResult};

pub(crate) fn by_name<'a, R: Read + Seek>(
    archive: &'a mut zip::ZipArchive<R>,
    name: &str,
) -> ZipResult<zip::read::ZipFile<'a, R>> {
    let index = match archive.index_for_name(name) {
        Some(index) => index,
        None => {
            let mut found = None;
            for (index, entry) in archive.file_names().enumerate() {
                if entry? == name {
                    found = Some(index);
                    break;
                }
            }
            found.ok_or(ZipError::FileNotFound)?
        }
    };
    archive.by_index(index)
}
