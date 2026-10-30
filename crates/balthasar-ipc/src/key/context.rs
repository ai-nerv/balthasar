use std::io::Write;

struct Bound(usize);

impl Write for Bound {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        if self.0 > 256 {
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn bounded(value: &serde_json::Value) -> bool {
    serde_json::to_writer(Bound(0), value).is_ok()
}

#[cfg(test)]
mod tests;
