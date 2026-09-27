//! macOS bookmarks: an opaque token for a file that keeps finding it after the
//! file or its folder is moved or renamed. Elsewhere, the path is all there is.

use std::path::{Path, PathBuf};

/// Hex, to sit in `links.json`. `None` if the file can't be bookmarked.
#[cfg(target_os = "macos")]
pub fn create(path: &Path) -> Option<String> {
    use objc2_foundation::{NSString, NSURL, NSURLBookmarkCreationOptions};
    let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
    let data = url
        .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            NSURLBookmarkCreationOptions::empty(),
            None,
            None,
        )
        .ok()?;
    Some(data.to_vec().iter().map(|b| format!("{b:02x}")).collect())
}

/// Where the bookmarked file is now, if it still exists.
#[cfg(target_os = "macos")]
pub fn resolve(hex: &str) -> Option<PathBuf> {
    use objc2::runtime::Bool;
    use objc2_foundation::{NSData, NSURL, NSURLBookmarkResolutionOptions};
    let bytes: Vec<u8> =
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect::<Option<_>>()?;
    let data = NSData::with_bytes(&bytes);
    let mut stale = Bool::NO;
    // SAFETY: `stale` outlives the call, which only writes a BOOL through it.
    let url = unsafe {
        NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
            &data,
            NSURLBookmarkResolutionOptions::WithoutUI | NSURLBookmarkResolutionOptions::WithoutMounting,
            None,
            &mut stale,
        )
    }
    .ok()?;
    let path = PathBuf::from(url.path()?.to_string());
    path.exists().then_some(path)
}

#[cfg(not(target_os = "macos"))]
pub fn create(_: &Path) -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn resolve(_: &str) -> Option<PathBuf> {
    None
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn finds_the_file_after_its_folder_is_renamed() {
        let root = std::env::temp_dir().join(format!("fabio-bookmark-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("shop")).unwrap();
        let file = root.join("shop/schema.dbml");
        std::fs::write(&file, "Table a {}").unwrap();

        let mark = super::create(&file).unwrap();
        std::fs::rename(root.join("shop"), root.join("shop-renamed")).unwrap();
        let found = super::resolve(&mark).unwrap();
        assert!(found.ends_with("shop-renamed/schema.dbml"), "{}", found.display());

        std::fs::remove_file(&found).unwrap();
        assert_eq!(super::resolve(&mark), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
