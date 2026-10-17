use std::{
  ops::Deref,
  path::{Path, PathBuf},
};

use url::Url;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AbsoluteFilePath(PathBuf);

impl AbsoluteFilePath {
  #[must_use]
  pub fn new(path: impl AsRef<Path>) -> Option<Self> {
    let path = path.as_ref();

    if !path.is_absolute() {
      return None;
    }

    Some(Self(path.to_path_buf()))
  }

  #[must_use]
  pub fn as_path(&self) -> &Path {
    &self.0
  }

  #[must_use]
  pub fn into_path_buf(self) -> PathBuf {
    self.0
  }

  #[must_use]
  pub fn to_url(&self) -> Url {
    Url::from_file_path(&self.0)
      .expect("This can only error if the path is not absolute which we checked")
  }
}

impl Deref for AbsoluteFilePath {
  type Target = Path;

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}
