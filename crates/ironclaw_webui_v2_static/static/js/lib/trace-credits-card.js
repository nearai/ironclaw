export function formatSignedCredit(value) {
  const numeric = Number(value) || 0;
  return `${numeric >= 0 ? "+" : ""}${numeric.toFixed(2)}`;
}

export function sidebarTraceCreditsSummary(credits) {
  if (!credits || !credits.enrolled) return null;
  return {
    final: formatSignedCredit(credits.final_credit),
    accepted: credits.submissions_accepted || 0,
    submitted: credits.submissions_submitted || 0,
    heldCount: credits.manual_review_hold_count || 0,
  };
}
