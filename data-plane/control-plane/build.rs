// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::fmt::Write as _;

fn main() {
    let protoc_path = protoc_bin_vendored::protoc_bin_path().unwrap();

    unsafe {
        #[allow(clippy::disallowed_methods)]
        std::env::set_var("PROTOC", protoc_path);
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let base = std::path::Path::new(&manifest_dir);

    let controller_proto = base.join("proto/controller/v1/controller.proto");
    let controlplane_proto = base.join("proto/controlplane/v1/controlplane.proto");

    if controller_proto.exists() && controlplane_proto.exists() {
        println!("cargo:rerun-if-changed={}", controller_proto.display());
        println!("cargo:rerun-if-changed={}", controlplane_proto.display());

        tonic_prost_build::configure()
            .out_dir("src/api/gen")
            .compile_protos(
                &[
                    controller_proto.to_str().unwrap(),
                    controlplane_proto.to_str().unwrap(),
                ],
                &[base.join("proto").to_str().unwrap()],
            )
            .unwrap();
    }

    generate_schema(&manifest_dir);
}

// ─── Schema types for PRAGMA introspection ────────────────────────────────────

#[derive(diesel::QueryableByName)]
struct TableName {
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
}

#[derive(diesel::QueryableByName)]
#[allow(dead_code)]
struct ColumnInfo {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    cid: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
    #[diesel(sql_type = diesel::sql_types::Text, column_name = "type")]
    col_type: String,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    notnull: i32,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    dflt_value: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pk: i32,
}

/// Map a SQLite column type string to its Diesel schema type.
/// Follows the same rules as `diesel print-schema` for SQLite.
fn sqlite_type_to_diesel(type_name: &str) -> &'static str {
    let t = type_name.to_lowercase();
    if t.contains("bool") {
        "Bool"
    } else if t == "integer" || t == "int" || t.contains("tiny") || t == "int4" {
        "Integer"
    } else if t.contains("int") {
        // bigint, int8, unsigned big int, …
        "BigInt"
        // spellchecker:off
    } else if t.contains("real") || t.contains("floa") || t.contains("doub") {
        // spellchecker:on
        "Double"
    } else if t.contains("blob") || t == "binary" {
        "Binary"
    } else {
        // TEXT, VARCHAR, CHAR, CLOB, and the empty-type SQLite default
        "Text"
    }
}

fn generate_schema(manifest_dir: &str) {
    use diesel::connection::SimpleConnection as _;
    use diesel::prelude::*;
    use diesel::sqlite::SqliteConnection;

    println!("cargo:rerun-if-changed=src/db/sqlite/migrations/");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let db_path = format!("{out_dir}/schema_gen.db");
    let _ = std::fs::remove_file(&db_path);

    let up_sql = std::fs::read_to_string(format!(
        "{manifest_dir}/src/db/sqlite/migrations/00000000000000_initial/up.sql"
    ))
    .expect("failed to read up.sql");

    let mut conn =
        SqliteConnection::establish(&db_path).expect("failed to open schema generation db");
    conn.batch_execute(&up_sql)
        .expect("failed to apply schema DDL");

    let tables: Vec<TableName> = diesel::sql_query(
        "SELECT name FROM sqlite_master \
         WHERE type='table' AND name NOT LIKE 'sqlite_%' \
         ORDER BY name",
    )
    .load(&mut conn)
    .expect("failed to list tables");

    let mut out = String::new();
    writeln!(
        out,
        "// @generated — do not edit manually; regenerated from migrations/ on every build"
    )
    .unwrap();
    writeln!(
        out,
        "// Copyright AGNTCY Contributors (https://github.com/agntcy)"
    )
    .unwrap();
    writeln!(out, "// SPDX-License-Identifier: Apache-2.0").unwrap();
    writeln!(out).unwrap();

    for table in &tables {
        let cols: Vec<ColumnInfo> =
            diesel::sql_query(format!("PRAGMA table_info(\"{}\")", table.name))
                .load(&mut conn)
                .expect("failed to query table_info");

        // Collect primary key columns in order.
        let mut pk_cols: Vec<(i32, &str)> = cols
            .iter()
            .filter(|c| c.pk > 0)
            .map(|c| (c.pk, c.name.as_str()))
            .collect();
        pk_cols.sort_by_key(|(pk, _)| *pk);
        let pk_list = pk_cols
            .iter()
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join(", ");

        writeln!(out, "diesel::table! {{").unwrap();
        writeln!(out, "    {} ({}) {{", table.name, pk_list).unwrap();

        for col in &cols {
            let inner = sqlite_type_to_diesel(&col.col_type);
            let diesel_type = if col.notnull == 1 {
                inner.to_string()
            } else {
                format!("Nullable<{inner}>")
            };
            writeln!(out, "        {} -> {},", col.name, diesel_type).unwrap();
        }

        writeln!(out, "    }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    std::fs::write(format!("{out_dir}/schema.rs"), out).expect("failed to write schema.rs");
}
