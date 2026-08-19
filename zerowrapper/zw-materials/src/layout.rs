use std::path::{Path, PathBuf};

use zw_db::templates::version_sid;

pub fn templates_root(whiteboard_dir: &Path) -> PathBuf {
    whiteboard_dir.join("templates")
}

pub fn template_dir(root: &Path, tpl: &str) -> PathBuf {
    root.join(tpl)
}

pub fn head_path(root: &Path, tpl: &str) -> PathBuf {
    template_dir(root, tpl).join("HEAD")
}

pub fn version_dir(root: &Path, tpl: &str, ver: i64) -> PathBuf {
    template_dir(root, tpl).join("versions").join(ver.to_string())
}

pub fn files_dir(root: &Path, tpl: &str, ver: i64) -> PathBuf {
    version_dir(root, tpl, ver).join("files")
}

pub fn manifest_path(root: &Path, tpl: &str, ver: i64) -> PathBuf {
    version_dir(root, tpl, ver).join("manifest.json")
}

pub fn changelog_path(root: &Path, tpl: &str, ver: i64) -> PathBuf {
    version_dir(root, tpl, ver).join("changelog.md")
}

pub fn diff_path(root: &Path, tpl: &str, ver: i64) -> PathBuf {
    version_dir(root, tpl, ver).join("diff.json")
}

pub fn clone_meta_dir(whiteboard_dir: &Path, sid: &str, tpl: &str, ver: i64) -> PathBuf {
    whiteboard_dir
        .join("sessions")
        .join(sid)
        .join("materials")
        .join(version_sid(tpl, ver))
}

pub fn clone_meta_path(whiteboard_dir: &Path, sid: &str, tpl: &str, ver: i64) -> PathBuf {
    clone_meta_dir(whiteboard_dir, sid, tpl, ver).join("mat.json")
}

pub fn clone_files_dir(whiteboard_dir: &Path, sid: &str, tpl: &str, ver: i64) -> PathBuf {
    clone_meta_dir(whiteboard_dir, sid, tpl, ver).join("files")
}

pub fn is_valid_template_id(tpl: &str) -> bool {
    !tpl.is_empty()
        && !tpl.chars().any(|c| c.is_whitespace() || c == '/' || c == '@' || c == '\\')
}
