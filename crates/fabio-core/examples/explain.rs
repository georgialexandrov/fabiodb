//! Prints the findings for a statement against the dev database.
//!     cargo run -p fabio-core --example explain -- "select * from perf.big where bucket = 7"

use fabio_core::{Db, PgTarget, Target};

#[tokio::main]
async fn main() {
    let sql = std::env::args().nth(1).expect("a statement");
    let target = Target::Postgres(PgTarget::from_url("postgres://fabio@localhost:54329/chinook").unwrap());
    let plan = Db::open(&target).await.unwrap().explain(&sql, true).await.unwrap();
    println!("execution {:?} ms", plan.execution_ms);
    for f in plan.findings {
        println!("{:?} {:?}: {}", f.severity, f.path, f.message);
    }
}
