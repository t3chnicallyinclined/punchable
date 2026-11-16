//! A gzip reader that reads a stream the way Python's `gzip.open(path, "rb")` does, which is what `backup.py` judges a
//! dump by: any number of members one after another; an empty file is an empty stream; after a member, zero bytes of
//! padding are skipped; anything else after a member that is not a gzip header is an error; a cut member or a wrong
//! CRC or length is an error.

use std::io::{BufRead, Read};

use flate2::bufread::GzDecoder;

/// Reads a (possibly multi-member) gzip stream from a buffered reader.
pub struct PyGzip<R: BufRead> {
    current: Option<GzDecoder<R>>,
    between: Option<R>,
    after_member: bool,
    done: bool,
}

impl<R: BufRead> PyGzip<R> {
    /// Start reading `r`.
    pub fn new(r: R) -> Self {
        Self {
            current: None,
            between: Some(r),
            after_member: false,
            done: false,
        }
    }
}

impl<R: BufRead> Read for PyGzip<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.done || out.is_empty() {
                return Ok(0);
            }
            if self.current.is_none() {
                let Some(mut r) = self.between.take() else {
                    self.done = true;
                    return Ok(0);
                };
                loop {
                    let buf = r.fill_buf()?;
                    if buf.is_empty() {
                        self.done = true;
                        return Ok(0);
                    }
                    if !self.after_member {
                        break; // the first header: no padding is skipped before it
                    }
                    let zeros = buf.iter().take_while(|b| **b == 0).count();
                    let all = zeros == buf.len();
                    r.consume(zeros);
                    if !all {
                        break;
                    }
                }
                self.current = Some(GzDecoder::new(r));
            }
            let n = match self.current.as_mut() {
                Some(g) => g.read(out)?,
                None => 0,
            };
            if n > 0 {
                return Ok(n);
            }
            // the member is complete (its CRC and length matched)
            self.between = self.current.take().map(GzDecoder::into_inner);
            self.after_member = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Write};

    use flate2::{Compression, GzBuilder};

    use super::*;

    fn gz(data: &[u8]) -> Vec<u8> {
        let mut e = GzBuilder::new().mtime(0).write(Vec::new(), Compression::new(6));
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    fn read_all(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
        let mut out = Vec::new();
        PyGzip::new(BufReader::with_capacity(7, bytes)).read_to_end(&mut out)?;
        Ok(out)
    }

    #[test]
    fn an_empty_file_is_an_empty_stream() {
        assert_eq!(read_all(b"").unwrap(), b"");
    }

    #[test]
    fn members_follow_each_other_and_zero_padding_is_skipped() {
        let both = [gz(b"one;"), gz(b"two;")].concat();
        assert_eq!(read_all(&both).unwrap(), b"one;two;");
        let padded = [gz(b"one;"), vec![0u8; 20]].concat();
        assert_eq!(read_all(&padded).unwrap(), b"one;");
        let padded_between = [gz(b"one;"), vec![0u8; 20], gz(b"two;")].concat();
        assert_eq!(read_all(&padded_between).unwrap(), b"one;two;");
    }

    #[test]
    fn garbage_after_a_member_a_cut_member_a_bad_crc_and_leading_zeros_are_errors() {
        assert!(read_all(&[gz(b"one;"), b"garbage after the member".to_vec()].concat()).is_err());
        let g = gz(&b"abcdefgh".repeat(500));
        assert!(read_all(&g[..g.len() / 2]).is_err());
        let mut bad = g.clone();
        let n = bad.len();
        bad[n - 8] ^= 0xff;
        assert!(read_all(&bad).is_err());
        assert!(
            read_all(&[vec![0u8; 20], gz(b"one;")].concat()).is_err(),
            "no padding before the first header"
        );
        assert!(read_all(&[b'x'; 50]).is_err());
    }
}
