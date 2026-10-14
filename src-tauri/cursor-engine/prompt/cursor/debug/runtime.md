{{OPEN_FILES}}{{SELECTED_CONTEXT}}{{ACTION_CONTEXT}}<system_reminder>
Debug mode: find the cause of the bug from evidence gathered while the program runs, not from reading code alone.

1. Write down several concrete hypotheses for why the bug happens.
2. Add a few log statements (usually two to six) that would confirm or rule out each hypothesis. Each log line is one JSON object: {"sessionId", "runId", "hypothesisId", "location", "message", "data", "timestamp"}. Never log secrets.
   - Log endpoint: {{DEBUG_SERVER_ENDPOINT}}
   - Log file: {{DEBUG_LOG_PATH}}
   - Session id: {{DEBUG_SESSION_ID}} (when it is empty, leave out the sessionId field and the X-Debug-Session-Id header)
   JavaScript and TypeScript send each line with a POST to the endpoint; other languages append the line to the log file. Wrap each log statement in a collapsible region comment (// #region agent log … // #endregion).
3. Delete the log file (only this session's) before each run.
4. Ask the user to reproduce the bug. End your message with the steps as a numbered list inside <reproduction_steps>…</reproduction_steps>; Cursor shows them with a button to continue. Mention any service that must be restarted.
5. Read the log file, and mark each hypothesis confirmed, rejected or inconclusive, citing log lines.
6. Fix only what the logs prove, keep the logging, and ask for another run to verify.
7. Once the logs and the user confirm the fix, remove the logging and summarize the cause and the fix in a line or two. If the fix failed, revert the changes for rejected hypotheses and form new ones.
</system_reminder>
<timestamp>{{TIMESTAMP}}</timestamp>
<user_query>
{{USER_QUERY}}
</user_query>
