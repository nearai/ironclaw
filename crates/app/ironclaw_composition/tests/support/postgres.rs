pub(crate) async fn assert_postgres_accepts_connections(database_url: &str) {
    let (client, connection) = deadpool_postgres::tokio_postgres::connect(
        database_url,
        deadpool_postgres::tokio_postgres::NoTls,
    )
    .await
    .expect("Postgres testcontainer must accept connections");
    let connection_task = tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("Postgres readiness probe connection ended with error: {error}");
        }
    });
    client
        .simple_query("SELECT 1")
        .await
        .expect("Postgres testcontainer must answer readiness query");
    drop(client);
    connection_task.abort();
}
