use tauri_appnebula_lib::memory::librarian::Librarian;
use tauri_appnebula_lib::memory::sqlite_manager::TaskRow;

#[tokio::test]
async fn update_tasks_round_trip_writes_and_formats() {
    let dir = std::env::temp_dir().join(format!(
        "nebula_tasks_int_{}",
        uuid::Uuid::new_v4()
    ));
    let lib = Librarian::new(&dir).unwrap();
    lib.sqlite
        .conn
        .execute(
            "INSERT INTO conversations (id, title, created_at) VALUES ('conv-1','Test','2026-05-20T00:00:00Z')",
            [],
        )
        .unwrap();

    // First update: 3 tasks.
    let initial = vec![
        TaskRow {
            content: "Look up the data model".into(),
            active_form: "Looking up the data model".into(),
            status: "completed".into(),
        },
        TaskRow {
            content: "Build the parser".into(),
            active_form: "Building the parser".into(),
            status: "in_progress".into(),
        },
        TaskRow {
            content: "Wire up the UI".into(),
            active_form: "Wiring up the UI".into(),
            status: "pending".into(),
        },
    ];
    let saved = lib.set_tasks("conv-1", &initial).unwrap();
    assert_eq!(saved.len(), 3);

    // Read back via Librarian — positions correct, status preserved.
    let got = lib.get_tasks("conv-1").unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(got[1].status, "in_progress");
    assert_eq!(got[2].position, 2);

    // Context formatter uses active_form for the in-progress row.
    let ctx = lib.format_tasks_for_context("conv-1").unwrap().unwrap();
    assert!(ctx.contains("Building the parser"));
    assert!(ctx.contains("[\u{2713}] Look up the data model"));
    assert!(ctx.contains("[ ] Wire up the UI"));

    // Second update: replace-all.
    let replacement = vec![TaskRow {
        content: "All done".into(),
        active_form: "Doing all done".into(),
        status: "completed".into(),
    }];
    lib.set_tasks("conv-1", &replacement).unwrap();
    let got2 = lib.get_tasks("conv-1").unwrap();
    assert_eq!(got2.len(), 1);
    assert_eq!(got2[0].content, "All done");
}
