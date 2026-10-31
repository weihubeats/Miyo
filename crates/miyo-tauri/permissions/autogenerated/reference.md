## Default Permission

Miyo 核心插件的默认权限：允许前端调用全部 miyo 命令
（项目 / 接口 / 文件夹 / 环境 / cURL 解析 / 请求执行）。
命令的 allow-* 权限由 tauri-build 依据 invoke_handler 自动生成。

#### This default permission set includes the following:

- `allow-get-projects`
- `allow-save-project`
- `allow-update-projects-order`
- `allow-delete-project`
- `allow-set-active-project`
- `allow-get-active-project`
- `allow-list-project-stats`
- `allow-list-endpoints`
- `allow-get-endpoint`
- `allow-save-endpoint`
- `allow-delete-endpoint`
- `allow-duplicate-endpoint`
- `allow-list-folders`
- `allow-save-folder`
- `allow-delete-folder`
- `allow-parse-curl-command`
- `allow-list-environments`
- `allow-save-environment`
- `allow-set-active-environment`
- `allow-get-active-environment`
- `allow-delete-environment`
- `allow-export-environment`
- `allow-import-environment`
- `allow-get-global-variables`
- `allow-save-global-variables`
- `allow-get-global-params`
- `allow-save-global-params`
- `allow-execute-request`
- `allow-cancel-request`
- `allow-get-response-body`
- `allow-list-examples`
- `allow-save-example`
- `allow-delete-example`
- `allow-oauth-authorize`
- `allow-oauth-access-token`
- `allow-oauth-fetch-token`
- `allow-get-tls-settings`
- `allow-set-tls-settings`
- `allow-get-mcp-tools`
- `allow-set-mcp-tool-enabled`
- `allow-list-scenarios`
- `allow-save-scenario`
- `allow-delete-scenario`
- `allow-run-scenario`
- `allow-cancel-scenario-run`
- `allow-list-scenario-runs`
- `allow-get-scenario-run`
- `allow-codegen-render`
- `allow-cookie-list`
- `allow-cookie-clear`
- `allow-clipboard-write-text`
- `allow-list-request-histories`
- `allow-clear-request-histories`
- `allow-delete-request-history`
- `allow-mock-start`
- `allow-mock-stop`
- `allow-mock-status`
- `allow-mock-request-logs`
- `allow-mock-clear-logs`
- `allow-mock-reload`
- `allow-agent-start`
- `allow-agent-stop`
- `allow-agent-status`
- `allow-get-mcp-enabled`
- `allow-set-mcp-enabled`
- `allow-get-mcp-port`
- `allow-set-mcp-port`
- `allow-backup-export`
- `allow-backup-restore`
- `allow-import-document`
- `allow-read-text-file`
- `allow-export-openapi`
- `allow-export-docs`
- `allow-export-smoke-docs`
- `allow-save-text-file`
- `allow-test-endpoint`
- `allow-load-test`
- `allow-cancel-load-test`
- `allow-test-collection`
- `allow-cancel-test-collection`
- `allow-log-files`
- `allow-log-tail`
- `allow-log-dir-path`
- `allow-get-log-retention-days`
- `allow-set-log-retention-days`
- `allow-grpc-list-services`
- `allow-grpc-invoke`
- `allow-grpc-stream-close`
- `allow-grpc-list-proto-files`
- `allow-grpc-save-proto-files`
- `allow-grpc-delete-proto-file`
- `allow-ws-connect`
- `allow-ws-send`
- `allow-ws-disconnect`
- `allow-sse-connect`
- `allow-sse-disconnect`
- `allow-list-mock-rules`
- `allow-save-mock-rule`
- `allow-delete-mock-rule`
- `allow-get-http-proxy`
- `allow-set-http-proxy`
- `allow-get-http-timeout-ms`
- `allow-set-http-timeout-ms`
- `allow-get-data-dir`
- `allow-get-default-data-dir`
- `allow-set-data-dir`
- `allow-reset-data-dir`
- `allow-list-seq-counters`
- `allow-set-seq-counter`
- `allow-delete-seq-counter`
- `allow-test-http-proxy`
- `allow-list-request-examples`
- `allow-save-request-example`
- `allow-delete-request-example`
- `allow-list-test-cases`
- `allow-save-test-case`
- `allow-update-test-case-meta`
- `allow-set-test-case-folder`
- `allow-rename-test-case-folder`
- `allow-update-test-case-content`
- `allow-update-test-case-status`
- `allow-delete-test-case`
- `allow-list-teams`
- `allow-create-team`
- `allow-update-team`
- `allow-delete-team`
- `allow-move-project-to-team`
- `allow-sync-remote-list`
- `allow-sync-remote-create`
- `allow-sync-remote-delete`
- `allow-sync-push`
- `allow-sync-pull-preview`
- `allow-sync-pull-apply`
- `allow-sync-remote-list-projects`
- `allow-sync-import-project`

## Permission Table

<table>
<tr>
<th>Identifier</th>
<th>Description</th>
</tr>


<tr>
<td>

`miyo-tauri:allow-agent-start`

</td>
<td>

Enables the agent_start command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-agent-start`

</td>
<td>

Denies the agent_start command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-agent-status`

</td>
<td>

Enables the agent_status command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-agent-status`

</td>
<td>

Denies the agent_status command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-agent-stop`

</td>
<td>

Enables the agent_stop command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-agent-stop`

</td>
<td>

Denies the agent_stop command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-backup-export`

</td>
<td>

Enables the backup_export command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-backup-export`

</td>
<td>

Denies the backup_export command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-backup-restore`

</td>
<td>

Enables the backup_restore command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-backup-restore`

</td>
<td>

Denies the backup_restore command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-cancel-load-test`

</td>
<td>

Enables the cancel_load_test command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-cancel-load-test`

</td>
<td>

Denies the cancel_load_test command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-cancel-request`

</td>
<td>

Enables the cancel_request command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-cancel-request`

</td>
<td>

Denies the cancel_request command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-cancel-scenario-run`

</td>
<td>

Enables the cancel_scenario_run command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-cancel-scenario-run`

</td>
<td>

Denies the cancel_scenario_run command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-cancel-test-collection`

</td>
<td>

Enables the cancel_test_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-cancel-test-collection`

</td>
<td>

Denies the cancel_test_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-clear-request-histories`

</td>
<td>

Enables the clear_request_histories command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-clear-request-histories`

</td>
<td>

Denies the clear_request_histories command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-clipboard-write-text`

</td>
<td>

Enables the clipboard_write_text command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-clipboard-write-text`

</td>
<td>

Denies the clipboard_write_text command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-codegen-render`

</td>
<td>

Enables the codegen_render command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-codegen-render`

</td>
<td>

Denies the codegen_render command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-cookie-clear`

</td>
<td>

Enables the cookie_clear command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-cookie-clear`

</td>
<td>

Denies the cookie_clear command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-cookie-list`

</td>
<td>

Enables the cookie_list command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-cookie-list`

</td>
<td>

Denies the cookie_list command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-create-team`

</td>
<td>

Enables the create_team command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-create-team`

</td>
<td>

Denies the create_team command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-delete-endpoint`

</td>
<td>

Enables the delete_endpoint command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-delete-endpoint`

</td>
<td>

Denies the delete_endpoint command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-delete-environment`

</td>
<td>

Enables the delete_environment command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-delete-environment`

</td>
<td>

Denies the delete_environment command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-delete-example`

</td>
<td>

Enables the delete_example command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-delete-example`

</td>
<td>

Denies the delete_example command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-delete-folder`

</td>
<td>

Enables the delete_folder command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-delete-folder`

</td>
<td>

Denies the delete_folder command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-delete-mock-rule`

</td>
<td>

Enables the delete_mock_rule command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-delete-mock-rule`

</td>
<td>

Denies the delete_mock_rule command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-delete-project`

</td>
<td>

Enables the delete_project command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-delete-project`

</td>
<td>

Denies the delete_project command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-delete-request-example`

</td>
<td>

Enables the delete_request_example command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-delete-request-example`

</td>
<td>

Denies the delete_request_example command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-delete-request-history`

</td>
<td>

Enables the delete_request_history command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-delete-request-history`

</td>
<td>

Denies the delete_request_history command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-delete-scenario`

</td>
<td>

Enables the delete_scenario command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-delete-scenario`

</td>
<td>

Denies the delete_scenario command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-delete-seq-counter`

</td>
<td>

Enables the delete_seq_counter command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-delete-seq-counter`

</td>
<td>

Denies the delete_seq_counter command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-delete-team`

</td>
<td>

Enables the delete_team command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-delete-team`

</td>
<td>

Denies the delete_team command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-delete-test-case`

</td>
<td>

Enables the delete_test_case command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-delete-test-case`

</td>
<td>

Denies the delete_test_case command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-duplicate-endpoint`

</td>
<td>

Enables the duplicate_endpoint command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-duplicate-endpoint`

</td>
<td>

Denies the duplicate_endpoint command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-execute-request`

</td>
<td>

Enables the execute_request command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-execute-request`

</td>
<td>

Denies the execute_request command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-export-docs`

</td>
<td>

Enables the export_docs command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-export-docs`

</td>
<td>

Denies the export_docs command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-export-environment`

</td>
<td>

Enables the export_environment command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-export-environment`

</td>
<td>

Denies the export_environment command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-export-openapi`

</td>
<td>

Enables the export_openapi command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-export-openapi`

</td>
<td>

Denies the export_openapi command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-export-smoke-docs`

</td>
<td>

Enables the export_smoke_docs command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-export-smoke-docs`

</td>
<td>

Denies the export_smoke_docs command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-active-environment`

</td>
<td>

Enables the get_active_environment command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-active-environment`

</td>
<td>

Denies the get_active_environment command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-active-project`

</td>
<td>

Enables the get_active_project command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-active-project`

</td>
<td>

Denies the get_active_project command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-data-dir`

</td>
<td>

Enables the get_data_dir command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-data-dir`

</td>
<td>

Denies the get_data_dir command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-default-data-dir`

</td>
<td>

Enables the get_default_data_dir command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-default-data-dir`

</td>
<td>

Denies the get_default_data_dir command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-endpoint`

</td>
<td>

Enables the get_endpoint command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-endpoint`

</td>
<td>

Denies the get_endpoint command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-global-params`

</td>
<td>

Enables the get_global_params command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-global-params`

</td>
<td>

Denies the get_global_params command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-global-variables`

</td>
<td>

Enables the get_global_variables command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-global-variables`

</td>
<td>

Denies the get_global_variables command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-http-proxy`

</td>
<td>

Enables the get_http_proxy command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-http-proxy`

</td>
<td>

Denies the get_http_proxy command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-http-timeout-ms`

</td>
<td>

Enables the get_http_timeout_ms command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-http-timeout-ms`

</td>
<td>

Denies the get_http_timeout_ms command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-log-retention-days`

</td>
<td>

Enables the get_log_retention_days command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-log-retention-days`

</td>
<td>

Denies the get_log_retention_days command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-mcp-enabled`

</td>
<td>

Enables the get_mcp_enabled command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-mcp-enabled`

</td>
<td>

Denies the get_mcp_enabled command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-mcp-port`

</td>
<td>

Enables the get_mcp_port command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-mcp-port`

</td>
<td>

Denies the get_mcp_port command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-mcp-tools`

</td>
<td>

Enables the get_mcp_tools command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-mcp-tools`

</td>
<td>

Denies the get_mcp_tools command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-projects`

</td>
<td>

Enables the get_projects command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-projects`

</td>
<td>

Denies the get_projects command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-response-body`

</td>
<td>

Enables the get_response_body command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-response-body`

</td>
<td>

Denies the get_response_body command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-scenario-run`

</td>
<td>

Enables the get_scenario_run command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-scenario-run`

</td>
<td>

Denies the get_scenario_run command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-get-tls-settings`

</td>
<td>

Enables the get_tls_settings command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-get-tls-settings`

</td>
<td>

Denies the get_tls_settings command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-grpc-delete-proto-file`

</td>
<td>

Enables the grpc_delete_proto_file command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-grpc-delete-proto-file`

</td>
<td>

Denies the grpc_delete_proto_file command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-grpc-invoke`

</td>
<td>

Enables the grpc_invoke command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-grpc-invoke`

</td>
<td>

Denies the grpc_invoke command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-grpc-list-proto-files`

</td>
<td>

Enables the grpc_list_proto_files command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-grpc-list-proto-files`

</td>
<td>

Denies the grpc_list_proto_files command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-grpc-list-services`

</td>
<td>

Enables the grpc_list_services command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-grpc-list-services`

</td>
<td>

Denies the grpc_list_services command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-grpc-save-proto-files`

</td>
<td>

Enables the grpc_save_proto_files command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-grpc-save-proto-files`

</td>
<td>

Denies the grpc_save_proto_files command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-grpc-stream-close`

</td>
<td>

Enables the grpc_stream_close command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-grpc-stream-close`

</td>
<td>

Denies the grpc_stream_close command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-import-document`

</td>
<td>

Enables the import_document command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-import-document`

</td>
<td>

Denies the import_document command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-import-environment`

</td>
<td>

Enables the import_environment command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-import-environment`

</td>
<td>

Denies the import_environment command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-list-endpoints`

</td>
<td>

Enables the list_endpoints command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-list-endpoints`

</td>
<td>

Denies the list_endpoints command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-list-environments`

</td>
<td>

Enables the list_environments command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-list-environments`

</td>
<td>

Denies the list_environments command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-list-examples`

</td>
<td>

Enables the list_examples command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-list-examples`

</td>
<td>

Denies the list_examples command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-list-folders`

</td>
<td>

Enables the list_folders command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-list-folders`

</td>
<td>

Denies the list_folders command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-list-mock-rules`

</td>
<td>

Enables the list_mock_rules command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-list-mock-rules`

</td>
<td>

Denies the list_mock_rules command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-list-project-stats`

</td>
<td>

Enables the list_project_stats command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-list-project-stats`

</td>
<td>

Denies the list_project_stats command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-list-request-examples`

</td>
<td>

Enables the list_request_examples command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-list-request-examples`

</td>
<td>

Denies the list_request_examples command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-list-request-histories`

</td>
<td>

Enables the list_request_histories command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-list-request-histories`

</td>
<td>

Denies the list_request_histories command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-list-scenario-runs`

</td>
<td>

Enables the list_scenario_runs command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-list-scenario-runs`

</td>
<td>

Denies the list_scenario_runs command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-list-scenarios`

</td>
<td>

Enables the list_scenarios command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-list-scenarios`

</td>
<td>

Denies the list_scenarios command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-list-seq-counters`

</td>
<td>

Enables the list_seq_counters command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-list-seq-counters`

</td>
<td>

Denies the list_seq_counters command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-list-teams`

</td>
<td>

Enables the list_teams command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-list-teams`

</td>
<td>

Denies the list_teams command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-list-test-cases`

</td>
<td>

Enables the list_test_cases command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-list-test-cases`

</td>
<td>

Denies the list_test_cases command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-load-test`

</td>
<td>

Enables the load_test command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-load-test`

</td>
<td>

Denies the load_test command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-log-dir-path`

</td>
<td>

Enables the log_dir_path command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-log-dir-path`

</td>
<td>

Denies the log_dir_path command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-log-files`

</td>
<td>

Enables the log_files command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-log-files`

</td>
<td>

Denies the log_files command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-log-tail`

</td>
<td>

Enables the log_tail command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-log-tail`

</td>
<td>

Denies the log_tail command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-mock-clear-logs`

</td>
<td>

Enables the mock_clear_logs command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-mock-clear-logs`

</td>
<td>

Denies the mock_clear_logs command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-mock-reload`

</td>
<td>

Enables the mock_reload command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-mock-reload`

</td>
<td>

Denies the mock_reload command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-mock-request-logs`

</td>
<td>

Enables the mock_request_logs command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-mock-request-logs`

</td>
<td>

Denies the mock_request_logs command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-mock-start`

</td>
<td>

Enables the mock_start command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-mock-start`

</td>
<td>

Denies the mock_start command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-mock-status`

</td>
<td>

Enables the mock_status command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-mock-status`

</td>
<td>

Denies the mock_status command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-mock-stop`

</td>
<td>

Enables the mock_stop command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-mock-stop`

</td>
<td>

Denies the mock_stop command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-move-project-to-team`

</td>
<td>

Enables the move_project_to_team command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-move-project-to-team`

</td>
<td>

Denies the move_project_to_team command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-oauth-access-token`

</td>
<td>

Enables the oauth_access_token command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-oauth-access-token`

</td>
<td>

Denies the oauth_access_token command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-oauth-authorize`

</td>
<td>

Enables the oauth_authorize command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-oauth-authorize`

</td>
<td>

Denies the oauth_authorize command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-oauth-fetch-token`

</td>
<td>

Enables the oauth_fetch_token command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-oauth-fetch-token`

</td>
<td>

Denies the oauth_fetch_token command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-parse-curl-command`

</td>
<td>

Enables the parse_curl_command command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-parse-curl-command`

</td>
<td>

Denies the parse_curl_command command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-read-text-file`

</td>
<td>

Enables the read_text_file command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-read-text-file`

</td>
<td>

Denies the read_text_file command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-rename-test-case-folder`

</td>
<td>

Enables the rename_test_case_folder command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-rename-test-case-folder`

</td>
<td>

Denies the rename_test_case_folder command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-reset-data-dir`

</td>
<td>

Enables the reset_data_dir command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-reset-data-dir`

</td>
<td>

Denies the reset_data_dir command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-run-scenario`

</td>
<td>

Enables the run_scenario command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-run-scenario`

</td>
<td>

Denies the run_scenario command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-save-endpoint`

</td>
<td>

Enables the save_endpoint command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-save-endpoint`

</td>
<td>

Denies the save_endpoint command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-save-environment`

</td>
<td>

Enables the save_environment command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-save-environment`

</td>
<td>

Denies the save_environment command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-save-example`

</td>
<td>

Enables the save_example command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-save-example`

</td>
<td>

Denies the save_example command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-save-folder`

</td>
<td>

Enables the save_folder command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-save-folder`

</td>
<td>

Denies the save_folder command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-save-global-params`

</td>
<td>

Enables the save_global_params command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-save-global-params`

</td>
<td>

Denies the save_global_params command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-save-global-variables`

</td>
<td>

Enables the save_global_variables command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-save-global-variables`

</td>
<td>

Denies the save_global_variables command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-save-mock-rule`

</td>
<td>

Enables the save_mock_rule command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-save-mock-rule`

</td>
<td>

Denies the save_mock_rule command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-save-project`

</td>
<td>

Enables the save_project command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-save-project`

</td>
<td>

Denies the save_project command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-save-request-example`

</td>
<td>

Enables the save_request_example command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-save-request-example`

</td>
<td>

Denies the save_request_example command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-save-scenario`

</td>
<td>

Enables the save_scenario command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-save-scenario`

</td>
<td>

Denies the save_scenario command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-save-test-case`

</td>
<td>

Enables the save_test_case command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-save-test-case`

</td>
<td>

Denies the save_test_case command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-save-text-file`

</td>
<td>

Enables the save_text_file command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-save-text-file`

</td>
<td>

Denies the save_text_file command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-set-active-environment`

</td>
<td>

Enables the set_active_environment command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-set-active-environment`

</td>
<td>

Denies the set_active_environment command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-set-active-project`

</td>
<td>

Enables the set_active_project command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-set-active-project`

</td>
<td>

Denies the set_active_project command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-set-data-dir`

</td>
<td>

Enables the set_data_dir command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-set-data-dir`

</td>
<td>

Denies the set_data_dir command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-set-http-proxy`

</td>
<td>

Enables the set_http_proxy command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-set-http-proxy`

</td>
<td>

Denies the set_http_proxy command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-set-http-timeout-ms`

</td>
<td>

Enables the set_http_timeout_ms command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-set-http-timeout-ms`

</td>
<td>

Denies the set_http_timeout_ms command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-set-log-retention-days`

</td>
<td>

Enables the set_log_retention_days command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-set-log-retention-days`

</td>
<td>

Denies the set_log_retention_days command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-set-mcp-enabled`

</td>
<td>

Enables the set_mcp_enabled command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-set-mcp-enabled`

</td>
<td>

Denies the set_mcp_enabled command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-set-mcp-port`

</td>
<td>

Enables the set_mcp_port command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-set-mcp-port`

</td>
<td>

Denies the set_mcp_port command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-set-mcp-tool-enabled`

</td>
<td>

Enables the set_mcp_tool_enabled command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-set-mcp-tool-enabled`

</td>
<td>

Denies the set_mcp_tool_enabled command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-set-seq-counter`

</td>
<td>

Enables the set_seq_counter command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-set-seq-counter`

</td>
<td>

Denies the set_seq_counter command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-set-test-case-folder`

</td>
<td>

Enables the set_test_case_folder command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-set-test-case-folder`

</td>
<td>

Denies the set_test_case_folder command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-set-tls-settings`

</td>
<td>

Enables the set_tls_settings command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-set-tls-settings`

</td>
<td>

Denies the set_tls_settings command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-sse-connect`

</td>
<td>

Enables the sse_connect command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-sse-connect`

</td>
<td>

Denies the sse_connect command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-sse-disconnect`

</td>
<td>

Enables the sse_disconnect command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-sse-disconnect`

</td>
<td>

Denies the sse_disconnect command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-sync-import-project`

</td>
<td>

Enables the sync_import_project command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-sync-import-project`

</td>
<td>

Denies the sync_import_project command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-sync-pull-apply`

</td>
<td>

Enables the sync_pull_apply command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-sync-pull-apply`

</td>
<td>

Denies the sync_pull_apply command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-sync-pull-preview`

</td>
<td>

Enables the sync_pull_preview command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-sync-pull-preview`

</td>
<td>

Denies the sync_pull_preview command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-sync-push`

</td>
<td>

Enables the sync_push command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-sync-push`

</td>
<td>

Denies the sync_push command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-sync-remote-create`

</td>
<td>

Enables the sync_remote_create command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-sync-remote-create`

</td>
<td>

Denies the sync_remote_create command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-sync-remote-delete`

</td>
<td>

Enables the sync_remote_delete command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-sync-remote-delete`

</td>
<td>

Denies the sync_remote_delete command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-sync-remote-list`

</td>
<td>

Enables the sync_remote_list command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-sync-remote-list`

</td>
<td>

Denies the sync_remote_list command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-sync-remote-list-projects`

</td>
<td>

Enables the sync_remote_list_projects command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-sync-remote-list-projects`

</td>
<td>

Denies the sync_remote_list_projects command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-test-collection`

</td>
<td>

Enables the test_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-test-collection`

</td>
<td>

Denies the test_collection command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-test-endpoint`

</td>
<td>

Enables the test_endpoint command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-test-endpoint`

</td>
<td>

Denies the test_endpoint command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-test-http-proxy`

</td>
<td>

Enables the test_http_proxy command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-test-http-proxy`

</td>
<td>

Denies the test_http_proxy command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-update-projects-order`

</td>
<td>

Enables the update_projects_order command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-update-projects-order`

</td>
<td>

Denies the update_projects_order command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-update-team`

</td>
<td>

Enables the update_team command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-update-team`

</td>
<td>

Denies the update_team command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-update-test-case-content`

</td>
<td>

Enables the update_test_case_content command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-update-test-case-content`

</td>
<td>

Denies the update_test_case_content command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-update-test-case-meta`

</td>
<td>

Enables the update_test_case_meta command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-update-test-case-meta`

</td>
<td>

Denies the update_test_case_meta command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-update-test-case-status`

</td>
<td>

Enables the update_test_case_status command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-update-test-case-status`

</td>
<td>

Denies the update_test_case_status command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-ws-connect`

</td>
<td>

Enables the ws_connect command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-ws-connect`

</td>
<td>

Denies the ws_connect command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-ws-disconnect`

</td>
<td>

Enables the ws_disconnect command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-ws-disconnect`

</td>
<td>

Denies the ws_disconnect command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:allow-ws-send`

</td>
<td>

Enables the ws_send command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`miyo-tauri:deny-ws-send`

</td>
<td>

Denies the ws_send command without any pre-configured scope.

</td>
</tr>
</table>
