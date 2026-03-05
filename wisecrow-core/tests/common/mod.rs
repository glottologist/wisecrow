use sqlx::PgPool;
use std::fmt::Write as FmtWrite;

/// Connects to the test database and runs migrations.
///
/// Uses `TEST_DATABASE_URL` if set, otherwise falls back to the default
/// docker-compose credentials.
pub async fn test_pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url)
        .await
        .expect("Failed to connect to test database");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    pool
}

/// Truncates all data tables so each test starts from a clean state.
pub async fn truncate_tables(pool: &PgPool) {
    sqlx::query("TRUNCATE translations, languages CASCADE")
        .execute(pool)
        .await
        .expect("Failed to truncate tables");
}

/// Generates `n` unique (source, target) pair tuples.
pub fn make_pairs(n: usize) -> Vec<(String, String)> {
    (0..n)
        .map(|i| (format!("source_{i}"), format!("target_{i}")))
        .collect()
}

/// Generates a TMX document containing `pairs` of (source, target) text.
pub fn generate_tmx(pairs: &[(&str, &str)], src_lang: &str, tgt_lang: &str) -> String {
    let mut body = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<tmx version=\"1.4\">\n  <body>\n",
    );
    for (source, target) in pairs {
        write!(
            body,
            "    <tu>\n      <tuv xml:lang=\"{src_lang}\"><seg>{source}</seg></tuv>\n      <tuv xml:lang=\"{tgt_lang}\"><seg>{target}</seg></tuv>\n    </tu>\n"
        ).expect("String write is infallible");
    }
    body.push_str("  </body>\n</tmx>");
    body
}

/// Generates an OPUS XML alignment document containing `pairs`.
pub fn generate_xml_alignment(pairs: &[(&str, &str)], src_lang: &str, tgt_lang: &str) -> String {
    let mut doc = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<cesAlign>\n");
    for (source, target) in pairs {
        write!(
            doc,
            "  <linkGrp>\n    <s xml:lang=\"{src_lang}\">{source}</s>\n    <s xml:lang=\"{tgt_lang}\">{target}</s>\n  </linkGrp>\n"
        ).expect("String write is infallible");
    }
    doc.push_str("</cesAlign>");
    doc
}

/// Returns the count of rows in the `translations` table.
pub async fn count_translations(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM translations")
        .fetch_one(pool)
        .await
        .expect("Failed to count translations")
}

/// Returns the count of rows in the `languages` table.
pub async fn count_languages(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM languages")
        .fetch_one(pool)
        .await
        .expect("Failed to count languages")
}

/// Returns all (`from_phrase`, `to_phrase`) pairs ordered by `from_phrase`.
pub async fn get_translation_pairs(pool: &PgPool) -> Vec<(String, String)> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT from_phrase, to_phrase FROM translations ORDER BY from_phrase",
    )
    .fetch_all(pool)
    .await
    .expect("Failed to fetch translation pairs")
}
