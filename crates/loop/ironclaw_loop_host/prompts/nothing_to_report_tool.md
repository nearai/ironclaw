Complete a suppression-enabled scheduled run when it produced no result worth
delivering. Call this tool only when there is nothing to report, with arguments
matching its schema. A validated call ends the run without an assistant message.
When there is a result, do not call this tool; return the result as the ordinary
assistant response instead.
