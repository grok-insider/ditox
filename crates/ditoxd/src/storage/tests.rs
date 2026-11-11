use rusqlite::params;

use super::*;
use crate::image::tiny_png;
use crate::testing::{self, TempDir, TestStore};

fn ids(entries: &[Entry]) -> Vec<i64> {
    entries.iter().map(|entry| entry.id).collect()
}

fn all(store: &Storage) -> Vec<Entry> {
    store.list(&ListQuery::new("", Filter::All, 50)).unwrap()
}

fn search(store: &Storage, query: &str) -> Vec<i64> {
    ids(&store.list(&ListQuery::new(query, Filter::All, 20)).unwrap())
}

fn png(tag: &str) -> Vec<u8> {
    let mut bytes = tiny_png(4, 5);
    bytes.extend_from_slice(tag.as_bytes());
    bytes
}

fn age(store: &Storage, id: i64, ms: i64) {
    let at = store.now_ms().unwrap() - ms;
    store
        .raw()
        .execute(
            "UPDATE entries SET created_at_ms = ?, last_used_at_ms = NULL WHERE id = ?",
            params![at, id],
        )
        .unwrap();
}

fn image_offer<'a>(bytes: &'a [u8], preview: &'a str, formats: &'a [OfferFormat]) -> Offer<'a> {
    Offer {
        kind: Kind::Image,
        mime: "image/png",
        bytes,
        preview,
        source_app: None,
        formats,
        size: ImageSize::default(),
    }
}

#[test]
fn text_is_added_listed_found_pinned_and_deleted() {
    let TestStore {
        cfg,
        store,
        dir: _dir,
    } = testing::store("basic");
    let first = store.add_text(&cfg, "first clip").unwrap();
    let second = store.add_text(&cfg, "second \x1b[31mclip\x1b[0m").unwrap();
    assert_eq!(store.add_text(&cfg, "first clip").unwrap(), first);

    let entry = store.get(second).unwrap().unwrap();
    assert_eq!(entry.content, "second clip");
    assert_eq!(entry.kind, Kind::Text);
    assert_eq!(entry.mime, "text/plain;charset=utf-8");
    assert_eq!(entry.hash, text::sha256_hex(b"second clip"));
    assert_eq!(entry.byte_len, 11);
    assert_eq!(store.format_mimes(second).unwrap(), ["text/plain"]);
    assert_eq!(
        store.format_text(second, "text/plain").unwrap().as_deref(),
        Some("second clip")
    );

    assert_eq!(all(&store).len(), 2);
    assert_eq!(search(&store, "second"), [second]);
    assert!(store.favorite(first, true).unwrap());
    assert_eq!(
        ids(&store
            .list(&ListQuery::new("", Filter::Favorites, 10))
            .unwrap()),
        [first]
    );
    assert_eq!(all(&store)[0].id, first);

    assert!(store.delete(second).unwrap());
    assert!(!store.delete(second).unwrap());
    assert!(store.delete(first).unwrap());
    assert!(all(&store).is_empty());
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
}

#[test]
fn a_compact_list_cuts_long_text_and_get_returns_all_of_it() {
    let TestStore {
        cfg,
        store,
        dir: _dir,
    } = testing::store("compact");
    let long = "line of text\n".repeat(200);
    let id = store.add_text(&cfg, &long).unwrap();
    let mut query = ListQuery::new("", Filter::All, 10);
    query.compact = true;
    let listed = store.list(&query).unwrap();
    assert_eq!(listed[0].content.chars().count(), 400);
    assert_eq!(listed[0].byte_len, long.len() as i64);
    assert_eq!(store.get(id).unwrap().unwrap().content, long);
}

#[test]
fn an_image_is_stored_once_with_its_blob_and_size() {
    let TestStore {
        cfg,
        store,
        dir: _dir,
    } = testing::store("image");
    let bytes = png("one");
    let id = store
        .add_image(&cfg, "image/png", &bytes, image::size(&bytes, "image/png"))
        .unwrap();
    assert_eq!(
        store
            .add_image(&cfg, "image/png", &bytes, ImageSize::default())
            .unwrap(),
        id
    );

    let entry = store.get(id).unwrap().unwrap();
    let path = entry.blob_path.clone().unwrap();
    assert_eq!(entry.kind, Kind::Image);
    assert_eq!((entry.image_width, entry.image_height), (Some(4), Some(5)));
    assert_eq!(
        entry.preview,
        format!("Image image/png {} bytes 4x5", bytes.len())
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(store.format_mimes(id).unwrap(), ["image/png"]);
    assert_eq!(store.format_text(id, "image/png").unwrap(), None);
    assert_eq!(
        ids(&store.list(&ListQuery::new("", Filter::Images, 10)).unwrap()),
        [id]
    );

    assert!(store.delete(id).unwrap());
    assert!(!blob::file_exists(&path));
}

#[test]
fn an_offer_is_one_image_row_with_searchable_text_siblings() {
    let TestStore {
        cfg,
        store,
        dir: _dir,
    } = testing::store("offer");
    let image = png("both");
    let formats = [
        OfferFormat {
            mime: "image/png".into(),
            bytes: image.clone().into(),
            image: true,
        },
        OfferFormat {
            mime: "text/uri-list".into(),
            bytes: b"https://example.com/both".to_vec().into(),
            image: false,
        },
    ];
    let id = store
        .add_offer(
            &cfg,
            &image_offer(&image, "https://example.com/both", &formats),
        )
        .unwrap();
    let entry = store.get(id).unwrap().unwrap();
    assert_eq!(entry.kind, Kind::Image);
    assert!(entry.preview.contains("example.com/both"));
    assert_eq!(search(&store, "example.com/both"), [id]);
    assert_eq!(
        store.format_mimes(id).unwrap(),
        ["image/png", "text/uri-list"]
    );
    assert_eq!(
        store.format_text(id, "text/uri-list").unwrap().as_deref(),
        Some("https://example.com/both")
    );

    let again = [OfferFormat {
        mime: "image/png".into(),
        bytes: image.clone().into(),
        image: true,
    }];
    assert_eq!(
        store
            .add_offer(&cfg, &image_offer(&image, "", &again))
            .unwrap(),
        id
    );
    assert_eq!(store.format_mimes(id).unwrap(), ["image/png"]);
    assert_eq!(all(&store).len(), 1);
}

#[test]
fn a_sticky_row_leads_the_list_and_retention_does_not_keep_it() {
    let TestStore {
        mut cfg,
        store,
        dir: _dir,
    } = testing::store("sticky");
    let id = store.add_text(&cfg, "sticky text").unwrap();
    let newer = store.add_text(&cfg, "newer plain row").unwrap();
    age(&store, id, 60_000);
    assert!(store.set_sticky(id, true).unwrap());
    assert_eq!(ids(&all(&store)), [id, newer]);

    cfg.max_entries = 1;
    let newest = store.add_text(&cfg, "newest plain row").unwrap();
    assert_eq!(ids(&all(&store)), [newest]);
}

#[test]
fn like_patterns_escape_wildcards_and_fts_quotes_tokens() {
    assert_eq!(build_like_contains("50%_\\done"), "%50\\%\\_\\\\done%");
    assert_eq!(build_like_prefix("50%_\\done"), "50\\%\\_\\\\done%");
    assert_eq!(
        build_fts_query("say \"hi\" 'there'"),
        "\"say\" AND \"hi\" AND \"there\""
    );
    assert_eq!(build_fts_query("a\"b"), "\"a\"\"b\"");
    assert_eq!(build_fts_query("  "), "");

    let TestStore {
        cfg,
        store,
        dir: _dir,
    } = testing::store("like");
    let literal = store.add_text(&cfg, "progress 50%_done").unwrap();
    store.add_text(&cfg, "progress 50 percent done").unwrap();
    assert_eq!(search(&store, "50%_")[0], literal);
}

#[test]
fn a_v1_database_migrates_and_stays_searchable() {
    let dir = TempDir::new("migrate-v1");
    let cfg = Config::defaults(&dir.path("data"));
    std::fs::create_dir_all(&cfg.image_dir).unwrap();
    let raw = Connection::open(&cfg.db_path).unwrap();
    raw.execute_batch(&format!(
        "{V1_SCHEMA}
         INSERT INTO entries(kind, mime, content, preview, hash, favorite, created_at_ms, byte_len, source_app)
         VALUES ('text', 'text/plain;charset=utf-8', 'legacy searchable', 'legacy searchable', 'legacy-hash', 0, 1700000000000, 17, 'legacy-app');
         PRAGMA user_version = 1;"
    ))
    .unwrap();
    drop(raw);

    let store = Storage::open(&cfg).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    let entry = store.get(1).unwrap().unwrap();
    assert_eq!(entry.content, "legacy searchable");
    assert_eq!(entry.source_app.as_deref(), Some("legacy-app"));
    assert_eq!(
        (
            entry.blob_path,
            entry.image_width,
            entry.last_used_at_ms,
            entry.sticky
        ),
        (None, None, None, false)
    );
    assert_eq!(search(&store, "legacy"), [1]);
    assert_eq!(store.format_mimes(1).unwrap(), ["text/plain"]);

    let bytes = png("migrated");
    let image = store
        .add_image(&cfg, "image/png", &bytes, image::size(&bytes, "image/png"))
        .unwrap();
    assert!(store.get(image).unwrap().unwrap().blob_path.is_some());
}

#[test]
fn a_v3_image_row_gets_a_format_without_touching_its_blob() {
    let dir = TempDir::new("migrate-v3");
    let cfg = Config::defaults(&dir.path("data"));
    std::fs::create_dir_all(&cfg.image_dir).unwrap();
    let blob_path = format!("{}/already.png", cfg.image_dir);
    std::fs::write(&blob_path, png("legacy-blob")).unwrap();
    let raw = Connection::open(&cfg.db_path).unwrap();
    raw.execute_batch(&format!(
        "{V1_SCHEMA}
         ALTER TABLE entries ADD COLUMN blob_path TEXT;
         ALTER TABLE entries ADD COLUMN image_width INTEGER;
         ALTER TABLE entries ADD COLUMN image_height INTEGER;
         ALTER TABLE entries ADD COLUMN last_used_at_ms INTEGER;
         INSERT INTO entries(kind, mime, content, preview, hash, favorite, created_at_ms, byte_len, source_app, blob_path)
         VALUES ('image', 'image/png', 'legacy-hash', 'legacy image', 'legacy-hash', 0, 1700000000000, 30, 'old-app', '{blob_path}');
         PRAGMA user_version = 3;"
    ))
    .unwrap();
    drop(raw);

    let store = Storage::open(&cfg).unwrap();
    assert_eq!(store.format_mimes(1).unwrap(), ["image/png"]);
    assert_eq!(store.format_text(1, "image/png").unwrap(), None);
    assert_eq!(std::fs::read(&blob_path).unwrap(), png("legacy-blob"));
}

/// A history as 0.1 left it: schema 4, every watched text one line
/// break longer than what was copied.
fn zig_history(cfg: &Config) {
    {
        let store = Storage::open(cfg).unwrap();
        store.add_text(cfg, "ls -la\n").unwrap();
        store
            .add_text(cfg, "two lines\nand a real break\n\n")
            .unwrap();
        store.add_text(cfg, "\n").unwrap();
        store.add_text(cfg, "added by hand").unwrap();
        let image = png("with-link");
        let formats = [
            OfferFormat {
                mime: "image/png".into(),
                bytes: image.clone().into(),
                image: true,
            },
            OfferFormat {
                mime: "text/plain".into(),
                bytes: b"https://example.com/a\n".to_vec().into(),
                image: false,
            },
        ];
        store
            .add_offer(
                cfg,
                &image_offer(&image, "https://example.com/a\n", &formats),
            )
            .unwrap();
    }
    Connection::open(&cfg.db_path)
        .unwrap()
        .execute_batch("PRAGMA user_version = 4")
        .unwrap();
}

#[test]
fn a_schema_4_history_loses_the_line_break_the_old_watcher_added() {
    let dir = TempDir::new("migrate-v5");
    let cfg = Config::defaults(&dir.path("data"));
    std::fs::create_dir_all(&cfg.image_dir).unwrap();
    zig_history(&cfg);

    let store = Storage::open(&cfg).unwrap();
    assert_eq!(store.schema_version().unwrap(), 5);
    let texts: Vec<String> = (1..=4)
        .map(|id| store.get(id).unwrap().unwrap().content)
        .collect();
    assert_eq!(
        texts,
        [
            "ls -la",
            "two lines\nand a real break\n",
            "\n",
            "added by hand"
        ]
    );
    let first = store.get(1).unwrap().unwrap();
    assert_eq!(first.hash, text::sha256_hex(b"ls -la"));
    assert_eq!(first.byte_len, 6);
    assert_eq!(first.preview, "ls -la");
    assert_eq!(
        store.format_text(1, "text/plain").unwrap().as_deref(),
        Some("ls -la")
    );
    assert_eq!(
        store.format_text(5, "text/plain").unwrap().as_deref(),
        Some("https://example.com/a")
    );
    assert_eq!(search(&store, "ls -la"), [1]);

    assert_eq!(
        store.add_text(&cfg, "ls -la").unwrap(),
        1,
        "the same text copied again is the same clip"
    );
    drop(store);

    let again = Storage::open(&cfg).unwrap();
    assert_eq!(
        again.get(2).unwrap().unwrap().content,
        "two lines\nand a real break\n"
    );

    let before = Connection::open(format!("{}.before-v5", cfg.db_path)).unwrap();
    let kept: String = before
        .query_row("SELECT content FROM entries WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(kept, "ls -la\n");
    let version: i64 = before
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 4);
}

#[test]
fn a_new_history_gets_no_copy_and_an_image_added_again_keeps_its_text() {
    let TestStore {
        cfg,
        store,
        dir: _dir,
    } = testing::store("no-copy");
    assert!(!blob::file_exists(&format!("{}.before-v5", cfg.db_path)));

    let image = png("with-link");
    let formats = [
        OfferFormat {
            mime: "image/png".into(),
            bytes: image.clone().into(),
            image: true,
        },
        OfferFormat {
            mime: "text/uri-list".into(),
            bytes: b"https://example.com/a".to_vec().into(),
            image: false,
        },
    ];
    let id = store
        .add_offer(
            &cfg,
            &image_offer(&image, "https://example.com/a", &formats),
        )
        .unwrap();
    assert_eq!(
        store
            .add_image(&cfg, "image/png", &image, ImageSize::default())
            .unwrap(),
        id
    );
    assert_eq!(
        store.format_mimes(id).unwrap(),
        ["image/png", "text/uri-list"]
    );
    assert_eq!(search(&store, "example.com/a"), [id]);
}

#[test]
fn a_database_from_a_newer_ditox_is_refused() {
    let dir = TempDir::new("newer");
    let cfg = Config::defaults(&dir.path("data"));
    std::fs::create_dir_all(&cfg.data_dir).unwrap();
    Connection::open(&cfg.db_path)
        .unwrap()
        .execute_batch("PRAGMA user_version = 99")
        .unwrap();
    assert!(Storage::open(&cfg).is_err());
}

#[test]
fn text_that_is_not_utf8_still_reads() {
    let TestStore {
        cfg,
        store,
        dir: _dir,
    } = testing::store("lossy");
    let id = store.add_text(&cfg, "placeholder").unwrap();
    store
        .raw()
        .execute(
            "UPDATE entries SET preview = CAST(? AS TEXT) WHERE id = ?",
            params![&b"caf\xc3"[..], id],
        )
        .unwrap();
    let entry = store.get(id).unwrap().unwrap();
    assert_eq!(entry.preview, "caf\u{fffd}");
    assert_eq!(all(&store).len(), 1);
}

#[test]
fn search_ranks_exact_then_prefix_then_incidental() {
    let TestStore {
        cfg,
        store,
        dir: _dir,
    } = testing::store("rank");
    let exact = store.add_text(&cfg, "needle").unwrap();
    let contains = store.add_text(&cfg, "zzz needle newest").unwrap();
    let prefix = store.add_text(&cfg, "needle prefix match").unwrap();
    assert_eq!(search(&store, "needle"), [exact, prefix, contains]);
}

#[test]
fn search_includes_fuzzy_matches_and_ranks_acronyms_first() {
    let TestStore {
        cfg,
        store,
        dir: _dir,
    } = testing::store("fuzzy");
    let acronym = store.add_text(&cfg, "network latency error").unwrap();
    store.add_text(&cfg, "totally separate").unwrap();
    assert_eq!(search(&store, "nle"), [acronym]);
    let incidental = store.add_text(&cfg, "annular low effect").unwrap();
    assert_eq!(search(&store, "nle"), [acronym, incidental]);
}

#[test]
fn search_reaches_entries_outside_the_newest_page() {
    let TestStore {
        cfg,
        store,
        dir: _dir,
    } = testing::store("deep");
    let target = store.add_text(&cfg, "the one deep target").unwrap();
    for index in 0..30 {
        store
            .add_text(&cfg, &format!("filler row {index}"))
            .unwrap();
    }
    assert!(!ids(&store.list(&ListQuery::new("", Filter::All, 10)).unwrap()).contains(&target));
    assert_eq!(
        ids(&store
            .list(&ListQuery::new("deep target", Filter::All, 10))
            .unwrap()),
        [target]
    );
}

#[test]
fn retention_deletes_expired_rows_keeps_pinned_ones_and_removes_the_blob() {
    let TestStore {
        mut cfg,
        store,
        dir: _dir,
    } = testing::store("retention");
    let old_text = store.add_text(&cfg, "old text").unwrap();
    let pinned = store.add_text(&cfg, "old pinned").unwrap();
    assert!(store.favorite(pinned, true).unwrap());
    let image = store
        .add_image(&cfg, "image/png", &png("old"), ImageSize::default())
        .unwrap();
    let blob_path = store.get(image).unwrap().unwrap().blob_path.unwrap();
    for id in [old_text, pinned, image] {
        age(&store, id, 5000);
    }

    cfg.delete_after_seconds = 1;
    assert_eq!(store.apply_retention(&cfg).unwrap(), 2);
    assert_eq!(ids(&all(&store)), [pinned]);
    assert!(!blob::file_exists(&blob_path));
}

#[test]
fn the_image_budget_drops_the_oldest_unpinned_image_and_keeps_a_pinned_one() {
    let TestStore {
        mut cfg,
        store,
        dir: _dir,
    } = testing::store("budget");
    let (first, second, third) = (png("first-image"), png("second-image"), png("third-image"));
    let oldest = store
        .add_image(&cfg, "image/png", &first, ImageSize::default())
        .unwrap();
    let pinned = store
        .add_image(&cfg, "image/png", &second, ImageSize::default())
        .unwrap();
    age(&store, oldest, 60_000);
    age(&store, pinned, 120_000);
    assert!(store.favorite(pinned, true).unwrap());

    cfg.max_image_bytes = third.len() as u32;
    let newest = store
        .add_image(&cfg, "image/png", &third, ImageSize::default())
        .unwrap();
    let mut kept = ids(&all(&store));
    kept.sort_unstable();
    assert_eq!(kept, [pinned, newest]);
}

#[test]
fn a_shared_blob_lives_until_its_last_row_is_gone() {
    let TestStore {
        mut cfg,
        store,
        dir: _dir,
    } = testing::store("shared");
    cfg.allow_duplicates = true;
    let bytes = png("shared");
    let first = store
        .add_image(&cfg, "image/png", &bytes, ImageSize::default())
        .unwrap();
    let second = store
        .add_image(&cfg, "image/png", &bytes, ImageSize::default())
        .unwrap();
    assert_ne!(first, second);
    let path = store.get(first).unwrap().unwrap().blob_path.unwrap();
    assert!(store.delete(first).unwrap());
    assert!(blob::file_exists(&path));
    assert!(store.delete(second).unwrap());
    assert!(!blob::file_exists(&path));
}

#[test]
fn adding_an_image_again_cancels_its_stale_prune() {
    let TestStore {
        cfg,
        store,
        dir: _dir,
    } = testing::store("reprune");
    let bytes = png("again");
    let id = store
        .add_image(&cfg, "image/png", &bytes, ImageSize::default())
        .unwrap();
    let path = store.get(id).unwrap().unwrap().blob_path.unwrap();
    store
        .raw()
        .execute("DELETE FROM entry_formats WHERE entry_id = ?", params![id])
        .unwrap();
    store
        .raw()
        .execute("DELETE FROM entries WHERE id = ?", params![id])
        .unwrap();
    assert!(blob::file_exists(&path));

    let again = store
        .add_image(&cfg, "image/png", &bytes, ImageSize::default())
        .unwrap();
    let other = store
        .add_text(&cfg, "a delete runs the prune queue")
        .unwrap();
    assert!(store.delete(other).unwrap());
    assert!(blob::file_exists(&path));
    assert_eq!(
        store.get(again).unwrap().unwrap().blob_path.as_deref(),
        Some(path.as_str())
    );
}

#[test]
fn runtime_state_holds_the_pause_the_self_write_and_the_watcher_error() {
    let TestStore {
        store, dir: _dir, ..
    } = testing::store("runtime");
    assert!(!store.is_watcher_paused().unwrap());
    store.pause_watcher(0).unwrap();
    assert!(store.is_watcher_paused().unwrap());
    store.resume_watcher().unwrap();
    assert!(!store.is_watcher_paused().unwrap());
    store.pause_watcher(60_000).unwrap();
    assert!(store.is_watcher_paused().unwrap());
    store
        .raw()
        .execute(
            "UPDATE runtime_state SET value = '1' WHERE key = 'paused_until_ms'",
            [],
        )
        .unwrap();
    assert!(!store.is_watcher_paused().unwrap());

    assert_eq!(store.self_write_hash().unwrap(), None);
    store.mark_self_write("abc").unwrap();
    assert_eq!(store.self_write_hash().unwrap().as_deref(), Some("abc"));
    store.clear_self_write_hash().unwrap();
    assert_eq!(store.self_write_hash().unwrap(), None);

    store
        .set_watcher_error("clipboard: wl-paste exit 1")
        .unwrap();
    assert_eq!(
        store.watcher_last_error().unwrap().as_deref(),
        Some("clipboard: wl-paste exit 1")
    );
    store.clear_watcher_error().unwrap();
    assert_eq!(store.watcher_last_error().unwrap(), None);
}

#[test]
fn using_a_clip_moves_it_above_newer_ones() {
    let TestStore {
        cfg,
        store,
        dir: _dir,
    } = testing::store("used");
    let older = store.add_text(&cfg, "older entry").unwrap();
    let newer = store.add_text(&cfg, "newer entry").unwrap();
    age(&store, older, 60_000);
    age(&store, newer, 59_000);
    assert_eq!(ids(&all(&store)), [newer, older]);
    assert!(store.mark_used(older).unwrap());
    let listed = all(&store);
    assert_eq!(ids(&listed), [older, newer]);
    assert!(listed[0].last_used_at_ms.is_some());
}

#[test]
fn the_source_app_is_kept_and_refreshed_by_a_second_copy() {
    let TestStore {
        cfg,
        store,
        dir: _dir,
    } = testing::store("source");
    let id = store
        .add_text_from(&cfg, "same text", Some("firefox"))
        .unwrap();
    assert_eq!(
        store.get(id).unwrap().unwrap().source_app.as_deref(),
        Some("firefox")
    );
    assert_eq!(
        store
            .add_text_from(&cfg, "same text", Some("ghostty"))
            .unwrap(),
        id
    );
    assert_eq!(
        store.get(id).unwrap().unwrap().source_app.as_deref(),
        Some("ghostty")
    );
    assert_eq!(store.add_text_from(&cfg, "same text", None).unwrap(), id);
    assert_eq!(
        store.get(id).unwrap().unwrap().source_app.as_deref(),
        Some("ghostty")
    );
}

#[test]
fn a_long_paste_is_capped_and_a_second_copy_does_not_grow_it() {
    let TestStore {
        mut cfg,
        store,
        dir: _dir,
    } = testing::store("cap");
    cfg.max_text_bytes = 10;
    let long = "abcdefgh\u{f1}\u{f1}\u{f1}";
    let id = store.add_text(&cfg, long).unwrap();
    let stored = store.get(id).unwrap().unwrap();
    assert_eq!(stored.content, "abcdefgh\u{f1}");
    assert_eq!(store.add_text(&cfg, long).unwrap(), id);
    assert_eq!(all(&store).len(), 1);
}
