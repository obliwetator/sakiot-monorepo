// `sqlx::migrate!` in src/db.rs embeds the migrations at compile time, and
// cargo does not notice files added to that directory on its own. Without
// this, `cargo dev db up` keeps applying the old set after a new migration
// lands, until something else happens to rebuild the crate.
fn main() {
    println!("cargo:rerun-if-changed=../../sakiot-db/migrations");
}
