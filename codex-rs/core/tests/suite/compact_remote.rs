// NOTE: All tests in this file (remote_compact_replaces_history_for_followups,
// remote_compact_runs_automatically, remote_compact_persists_replacement_history_in_rollout)
// were removed because they use OpenAI ChatGPT authentication (CodexAuth::create_dummy_chatgpt_auth_for_testing).
// Azure Codex uses Azure Entra ID authentication instead and does not support ChatGPT tokens.
//
// Remote compaction functionality itself still exists in Azure Codex but these specific tests
// cannot run without the ChatGPT auth mechanism.
