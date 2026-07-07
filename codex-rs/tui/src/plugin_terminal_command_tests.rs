use super::*;
use ratatui::text::Line;

#[test]
fn resume_thread_result_uses_first_non_empty_stdout_line() {
    assert_eq!(
        interpret_plugin_terminal_stdout(
            PluginTerminalCommandResult::ResumeThread,
            "\n  thread-123  \nignored\n",
        ),
        PluginTerminalCommandOutcome::ResumeThread("thread-123".to_string())
    );
}

#[test]
fn text_result_splits_non_empty_stdout_lines() {
    assert_eq!(
        interpret_plugin_terminal_stdout(PluginTerminalCommandResult::None, "hello\nworld\n"),
        PluginTerminalCommandOutcome::Text(vec![Line::from("hello"), Line::from("world")])
    );
}
