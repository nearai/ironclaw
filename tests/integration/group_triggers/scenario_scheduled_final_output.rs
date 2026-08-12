//! A trusted scheduled-trigger run cannot report success after repeatedly
//! ending with a question for an absent user. The real runner issues its two
//! bounded completion nudges, then persists `invalid_model_output` while
//! retaining every rejected reply in the trigger thread.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_host_api::turn::{TurnOriginKind, TurnStatus};

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-triggered-invalid-final-output")
        .build()
        .await?;
    let questions = [
        "Which repository should I inspect?",
        "Should I inspect the main branch?",
        "Would you like me to continue?",
    ];

    let submission = h
        .submit_triggered_turn_scripted(
            "inspect the repository and report the result without asking questions",
            [
                RebornScriptedReply::text(questions[0]),
                RebornScriptedReply::text(questions[1]),
                RebornScriptedReply::text(questions[2]),
            ],
        )
        .await?;

    let state = h
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::Failed,
        )
        .await?;
    if state.product_context.as_ref().map(|context| context.origin)
        != Some(TurnOriginKind::ScheduledTrigger)
    {
        return Err("failed run lost its trusted ScheduledTrigger origin".into());
    }
    let failure = state
        .failure
        .as_ref()
        .ok_or("failed scheduled run missing failure evidence")?;
    if failure.category() != "invalid_model_output" {
        return Err(format!(
            "expected invalid_model_output, got {:?}",
            failure.category()
        )
        .into());
    }

    let history = h
        .thread_harness
        .history(submission.turn_scope.thread_id.clone())
        .await?;
    for question in questions {
        if !history.iter().any(|message| {
            message
                .content
                .as_deref()
                .is_some_and(|content| content.contains(question))
        }) {
            return Err(format!("rejected reply was not retained: {question:?}").into());
        }
    }
    Ok(())
}
