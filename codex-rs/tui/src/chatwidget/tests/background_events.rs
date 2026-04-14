use super::*;
use codex_protocol::protocol::ExecOutputStream;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn background_event_updates_status_header() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;

    chat.handle_codex_event(Event {
        id: "bg-1".into(),
        msg: EventMsg::BackgroundEvent(BackgroundEventEvent {
            message: "Waiting for `vim`".to_string(),
        }),
    });

    assert!(chat.bottom_pane.status_indicator_visible());
    assert_eq!(chat.current_status.header, "Waiting for `vim`");
    assert!(drain_insert_history(&mut rx).is_empty());
}

#[tokio::test]
async fn unified_exec_monitor_ready_emits_info_and_updates_monitors_output() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;

    begin_unified_exec_startup(&mut chat, "call-1", "4242", "npm run dev");
    chat.handle_codex_event(Event {
        id: "delta-1".into(),
        msg: EventMsg::ExecCommandOutputDelta(ExecCommandOutputDeltaEvent {
            call_id: "call-1".to_string(),
            stream: ExecOutputStream::Stdout,
            chunk: b"VITE ready in 250ms\n".to_vec(),
        }),
    });

    assert_eq!(chat.unified_exec_processes.len(), 1);
    assert_eq!(
        chat.unified_exec_processes[0].monitor.status(),
        crate::unified_exec_monitor::UnifiedExecMonitorStatus::Ready
    );
    assert_eq!(
        chat.unified_exec_processes[0].recent_chunks,
        vec!["VITE ready in 250ms".to_string()]
    );

    let cells = drain_insert_history(&mut rx);
    let rendered = cells
        .iter()
        .map(|lines| lines_to_single_string(lines))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        rendered.contains("background terminal 4242 is ready"),
        "expected ready notification, got {rendered:?}"
    );

    chat.add_monitors_output();
    let cells = drain_insert_history(&mut rx);
    let rendered = cells
        .iter()
        .map(|lines| lines_to_single_string(lines))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("Monitored background terminals"));
    assert!(rendered.contains("#4242 [READY]"));
    assert!(rendered.contains("ready: VITE ready in 250ms"));
}

#[tokio::test]
async fn unified_exec_monitor_error_notifies_once_per_signature() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;

    begin_unified_exec_startup(&mut chat, "call-1", "4242", "npm run dev");
    for event_id in ["delta-1", "delta-2"] {
        chat.handle_codex_event(Event {
            id: event_id.into(),
            msg: EventMsg::ExecCommandOutputDelta(ExecCommandOutputDeltaEvent {
                call_id: "call-1".to_string(),
                stream: ExecOutputStream::Stderr,
                chunk: b"Error: failed to compile\n".to_vec(),
            }),
        });
    }

    assert_eq!(
        chat.unified_exec_processes[0].monitor.status(),
        crate::unified_exec_monitor::UnifiedExecMonitorStatus::Error
    );

    let cells = drain_insert_history(&mut rx);
    let rendered = cells
        .iter()
        .map(|lines| lines_to_single_string(lines))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        rendered.contains("reported an error"),
        "expected error notification, got {rendered:?}"
    );
    assert_eq!(
        rendered.matches("reported an error").count(),
        1,
        "expected the monitor to suppress duplicate error notifications"
    );
}
