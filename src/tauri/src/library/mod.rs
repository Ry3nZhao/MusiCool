//! 音乐库索引（SQLite）。
//!
//! 数据库位于 `<音乐库>/.musicool/library.db`，封面缓存在 `<音乐库>/.musicool/covers/`。
//! 一首歌在库里只占一条记录：有网易云歌曲 ID 的按 ID 去重，否则按文件路径去重。
//! 歌词（同名 `.lrc`）和封面不单独成条，作为歌曲的附属字段。
//!
//! 数据库只是缓存：所有字段都能从音频文件、标签和同名 `.lrc` 重新得到，
//! 删掉 `.musicool` 后刷新即可重建。

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const DATA_DIR: &str = ".musicool";
/// 改名为 MusiCool 前使用的数据目录
const LEGACY_DATA_DIR: &str = ".musiccool";
const SCHEMA_VERSION: i32 = 1;
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "m4a", "ogg", "wav"];
/// 写在注释标签里的网易云歌曲 ID 前缀，便于从文件本身恢复 ID
pub const MUSIC_ID_COMMENT_PREFIX: &str = "netease_music_id=";

/// 库中的一首歌。
#[derive(Debug, Clone, Serialize)]
pub struct Track {
    pub id: i64,
    pub path: String,
    /// 显示名：优先标签里的标题，否则用文件名
    pub name: String,
    pub file_name: String,
    pub music_id: Option<i64>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<u64>,
    pub format: String,
    pub size: u64,
    pub lyrics_path: Option<String>,
    pub cover_path: Option<String>,
}

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn mtime_of(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn data_dir(root: &Path) -> PathBuf {
    root.join(DATA_DIR)
}

/// 打开（必要时创建）音乐库数据库。导入线程和刷新可能并发，所以用 WAL + busy_timeout。
pub fn open(root: &Path) -> Result<Connection, String> {
    let dir = data_dir(root);
    let legacy = root.join(LEGACY_DATA_DIR);
    let migrated = !dir.exists() && legacy.is_dir() && std::fs::rename(&legacy, &dir).is_ok();
    std::fs::create_dir_all(dir.join("covers")).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let conn = Connection::open(dir.join("library.db")).map_err(|e| format!("打开数据库失败: {e}"))?;
    conn.busy_timeout(std::time::Duration::from_secs(10)).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL").map_err(|e| e.to_string())?;

    let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).map_err(|e| e.to_string())?;
    if version < SCHEMA_VERSION {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tracks (
                id            INTEGER PRIMARY KEY,
                path          TEXT    NOT NULL UNIQUE,
                music_id      INTEGER UNIQUE,
                title         TEXT,
                artist        TEXT,
                album         TEXT,
                duration_secs INTEGER,
                format        TEXT    NOT NULL,
                size          INTEGER NOT NULL,
                mtime         INTEGER NOT NULL,
                lyrics_path   TEXT,
                cover_path    TEXT,
                added_at      INTEGER NOT NULL,
                updated_at    INTEGER NOT NULL
            );
            PRAGMA user_version = 1;",
        )
        .map_err(|e| format!("初始化数据库失败: {e}"))?;
    }
    if migrated {
        // 封面路径是绝对路径，随数据目录一起改名
        conn.execute(
            "UPDATE tracks SET cover_path = replace(cover_path, ?1, ?2)",
            params![format!("/{LEGACY_DATA_DIR}/"), format!("/{DATA_DIR}/")],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(conn)
}

/// 从音频文件读出的信息。
struct Scanned {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    duration: Option<u64>,
    music_id: Option<i64>,
    picture: Option<Vec<u8>>,
}

fn read_audio(path: &Path) -> Scanned {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::tag::{Accessor, ItemKey};

    let mut s = Scanned { title: None, artist: None, album: None, duration: None, music_id: None, picture: None };
    let Ok(tagged) = lofty::read_from_path(path) else {
        return s;
    };
    s.duration = Some(tagged.properties().duration().as_secs());
    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        s.title = tag.title().map(|v| v.to_string()).filter(|v| !v.is_empty());
        s.artist = tag.artist().map(|v| v.to_string()).filter(|v| !v.is_empty());
        s.album = tag.album().map(|v| v.to_string()).filter(|v| !v.is_empty());
        s.music_id = tag
            .get_strings(ItemKey::Comment)
            .find_map(|c| c.strip_prefix(MUSIC_ID_COMMENT_PREFIX).and_then(|id| id.trim().parse().ok()));
        s.picture = tag.pictures().first().map(|p| p.data().to_vec());
    }
    s
}

/// 标签缺失时，从路径推断：`歌手/专辑/歌名.ext` 或文件名 `歌手 - 歌名`（网易云的下载命名规则）。
fn fill_from_path(s: &mut Scanned, root: &Path, path: &Path) {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let dirs: Vec<String> = rel
        .parent()
        .map(|p| p.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    let stem = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();

    // 有 `歌手/专辑/` 目录时以目录为准（歌名本身可能带 " - "）；根目录下的文件才按 `歌手 - 歌名` 拆
    let (artist, title) = match stem.split_once(" - ") {
        Some((a, t)) if dirs.len() < 2 => (Some(a.to_string()), t.to_string()),
        _ => (None, stem.clone()),
    };
    if s.title.is_none() {
        s.title = Some(title);
    }
    if s.artist.is_none() {
        s.artist = artist.or_else(|| (dirs.len() >= 2).then(|| dirs[dirs.len() - 2].clone()));
    }
    if s.album.is_none() && dirs.len() >= 2 {
        s.album = dirs.last().cloned();
    }
}

/// 封面缓存文件名：有歌曲 ID 用 ID，否则用相对路径的哈希（FNV-1a，跨版本稳定）。
fn cover_key(root: &Path, path: &Path, music_id: Option<i64>) -> String {
    if let Some(id) = music_id {
        return format!("track-{id}");
    }
    let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in rel.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("path-{h:016x}")
}

/// 索引（新增或更新）一个音频文件。返回 false 表示它和库里已有的歌重复，未单独入库。
pub fn index_file(conn: &Connection, root: &Path, path: &Path) -> Result<bool, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("读取文件信息失败 {path:?}: {e}"))?;
    let path_str = path.to_string_lossy().to_string();
    let mut s = read_audio(path);
    fill_from_path(&mut s, root, path);

    // 同一首歌（同 ID）已经在别的路径入库、且那个文件还在：视为重复，不再新增一条
    if let Some(id) = s.music_id {
        let other: Option<String> = conn
            .query_row("SELECT path FROM tracks WHERE music_id = ?1 AND path <> ?2", params![id, path_str], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(other) = other {
            if Path::new(&other).exists() {
                return Ok(false);
            }
            conn.execute("DELETE FROM tracks WHERE path = ?1", params![other]).map_err(|e| e.to_string())?;
        }
    }

    let lyrics = path.with_extension("lrc");
    let lyrics_path = lyrics.is_file().then(|| lyrics.to_string_lossy().to_string());

    let cover_path = match s.picture {
        Some(data) => {
            let ext = if data.starts_with(b"\x89PNG") { "png" } else { "jpg" };
            let cover = data_dir(root).join("covers").join(format!("{}.{ext}", cover_key(root, path, s.music_id)));
            if std::fs::metadata(&cover).map(|m| m.len() != data.len() as u64).unwrap_or(true) {
                std::fs::write(&cover, &data).map_err(|e| format!("写入封面失败: {e}"))?;
            }
            Some(cover.to_string_lossy().to_string())
        }
        None => None,
    };

    let format = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let ts = now();
    conn.execute(
        "INSERT INTO tracks (path, music_id, title, artist, album, duration_secs, format, size, mtime,
                             lyrics_path, cover_path, added_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)
         ON CONFLICT(path) DO UPDATE SET
            music_id = excluded.music_id, title = excluded.title, artist = excluded.artist,
            album = excluded.album, duration_secs = excluded.duration_secs, format = excluded.format,
            size = excluded.size, mtime = excluded.mtime, lyrics_path = excluded.lyrics_path,
            cover_path = excluded.cover_path, updated_at = excluded.updated_at",
        params![
            path_str, s.music_id, s.title, s.artist, s.album, s.duration.map(|d| d as i64), format,
            meta.len() as i64, mtime_of(&meta), lyrics_path, cover_path, ts
        ],
    )
    .map_err(|e| format!("写入数据库失败: {e}"))?;
    Ok(true)
}

/// 让数据库与磁盘一致：新增/变化的音频重新索引，已删除的移除，歌词有增删的更新。
pub fn sync(root: &Path) -> Result<(), String> {
    let mut conn = open(root)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    let known: std::collections::HashMap<String, (i64, i64, Option<String>)> = {
        let mut stmt = tx.prepare("SELECT path, size, mtime, lyrics_path FROM tracks").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))))
            .map_err(|e| e.to_string())?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let mut seen = std::collections::HashSet::new();
    let walker = WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.file_name() != DATA_DIR && e.file_name() != LEGACY_DATA_DIR);
    for entry in walker.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !entry.file_type().is_file() || !is_audio(path) {
            continue;
        }
        let path_str = path.to_string_lossy().to_string();
        seen.insert(path_str.clone());
        let Ok(meta) = entry.metadata() else { continue };
        let lyrics = path.with_extension("lrc");
        let lyrics_now = lyrics.is_file().then(|| lyrics.to_string_lossy().to_string());
        let unchanged = known.get(&path_str).is_some_and(|(size, mtime, lrc)| {
            *size == meta.len() as i64 && *mtime == mtime_of(&meta) && *lrc == lyrics_now
        });
        if !unchanged {
            if let Err(e) = index_file(&tx, root, path) {
                eprintln!("{e}");
            }
        }
    }

    for path in known.keys().filter(|p| !seen.contains(*p)) {
        tx.execute("DELETE FROM tracks WHERE path = ?1", params![path]).map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

pub fn list(root: &Path) -> Result<Vec<Track>, String> {
    let conn = open(root)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, path, title, music_id, artist, album, duration_secs, format, size, lyrics_path, cover_path
             FROM tracks ORDER BY COALESCE(artist, ''), COALESCE(title, path)",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let path: String = r.get(1)?;
            let p = Path::new(&path);
            let file_name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let title: Option<String> = r.get(2)?;
            let name = title.unwrap_or_else(|| p.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            Ok(Track {
                id: r.get(0)?,
                file_name,
                name,
                path,
                music_id: r.get(3)?,
                artist: r.get(4)?,
                album: r.get(5)?,
                duration: r.get::<_, Option<i64>>(6)?.map(|d| d as u64),
                format: r.get(7)?,
                size: r.get::<_, i64>(8)? as u64,
                lyrics_path: r.get(9)?,
                cover_path: r.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// 导入完成一个文件后调用：带上源目录里的同名歌词，并立即写入数据库。
pub fn on_imported(root: &Path, source: &Path, output: &Path) -> Result<(), String> {
    let src_lrc = source.with_extension("lrc");
    if src_lrc.is_file() {
        std::fs::copy(&src_lrc, output.with_extension("lrc")).map_err(|e| format!("复制歌词失败: {e}"))?;
    }
    let conn = open(root)?;
    index_file(&conn, root, output).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_fallback() {
        let root = Path::new("/lib");
        let empty = || Scanned { title: None, artist: None, album: None, duration: None, music_id: None, picture: None };
        let mut s = empty();
        fill_from_path(&mut s, root, Path::new("/lib/告五人/我肯定在几百年前就说过爱你/法兰西多士.flac"));
        assert_eq!((s.artist.as_deref(), s.album.as_deref(), s.title.as_deref()),
                   (Some("告五人"), Some("我肯定在几百年前就说过爱你"), Some("法兰西多士")));
        let mut s = empty();
        fill_from_path(&mut s, root, Path::new("/lib/陈奕迅 - 富士山下.flac"));
        assert_eq!((s.artist.as_deref(), s.album, s.title.as_deref()), (Some("陈奕迅"), None, Some("富士山下")));
        let mut s = empty();
        fill_from_path(&mut s, root, Path::new("/lib/Jon Schmidt/Michael Meets Mozart - 1 Piano/Michael Meets Mozart - 1 Piano.flac"));
        assert_eq!((s.artist.as_deref(), s.title.as_deref()), (Some("Jon Schmidt"), Some("Michael Meets Mozart - 1 Piano")));
    }

    #[test]
    fn cover_key_is_stable() {
        let root = Path::new("/lib");
        assert_eq!(cover_key(root, Path::new("/lib/a/b.flac"), Some(42)), "track-42");
        assert_eq!(
            cover_key(root, Path::new("/lib/a/b.flac"), None),
            cover_key(root, Path::new("/lib/a/b.flac"), None)
        );
    }
}

#[cfg(test)]
mod real_import {
    use crate::importer::rsync_like::plan_sync;
    use crate::importer::ImportParams;

    /// 端到端：按导入流程把真实网易云目录导入临时音乐库，再刷新并统计。
    /// `NCM_DIR=... LIB_DIR_TEST=... cargo test --release real_import -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn import_and_index() {
        let src = std::env::var("NCM_DIR").unwrap();
        let lib = std::path::PathBuf::from(std::env::var("LIB_DIR_TEST").unwrap());
        let params = ImportParams {
            input_path: src,
            output_path: lib.to_string_lossy().to_string(),
            exclude_dirs: vec![],
            thread_count: 1,
        };
        let plan = plan_sync(&params).unwrap();
        println!("planned={} skipped={}", plan.tasks.len(), plan.skipped_files);
        for t in &plan.tasks {
            let out = if t.is_ncm {
                let dir = t.output.parent().unwrap();
                let d = crate::decoder::ncm::decode_file(&t.input, dir, |_, _| {}).unwrap();
                let id = d.meta.as_ref().and_then(|m| m.music_id());
                let cover = id.and_then(|id| {
                    t.input.ancestors().skip(1).take(4)
                        .map(|p| p.join("meta").join(format!("track-{id}.jpg")))
                        .find(|p| p.is_file())
                        .map(|p| std::fs::read(p).unwrap())
                });
                crate::decoder::ncm::write_tags(&d, cover).unwrap();
                d.output_path
            } else {
                std::fs::create_dir_all(t.output.parent().unwrap()).unwrap();
                std::fs::copy(&t.input, &t.output).unwrap();
                t.output.clone()
            };
            super::on_imported(&lib, &t.input, &out).unwrap();
        }
        super::sync(&lib).unwrap();
        let tracks = super::list(&lib).unwrap();
        let ids: Vec<_> = tracks.iter().filter_map(|t| t.music_id).collect();
        let uniq: std::collections::HashSet<_> = ids.iter().collect();
        println!(
            "tracks={} with_id={} unique_ids={} lyrics={} cover={}",
            tracks.len(), ids.len(), uniq.len(),
            tracks.iter().filter(|t| t.lyrics_path.is_some()).count(),
            tracks.iter().filter(|t| t.cover_path.is_some()).count()
        );
        let non_audio = walkdir::WalkDir::new(&lib).into_iter().flatten()
            .filter(|e| e.file_type().is_file() && !e.path().components().any(|c| c.as_os_str() == ".musicool"))
            .filter(|e| !super::is_audio(e.path()) && e.path().extension().map_or(true, |x| x != "lrc"))
            .count();
        println!("non_audio_non_lrc_files={non_audio}");
        // 第二次刷新应当是幂等的
        super::sync(&lib).unwrap();
        assert_eq!(super::list(&lib).unwrap().len(), tracks.len());
        for t in tracks.iter().take(3) {
            println!("{:?} | {:?} | {:?} | id={:?} lrc={} cover={}", t.name, t.artist, t.album, t.music_id, t.lyrics_path.is_some(), t.cover_path.is_some());
        }
    }
}
