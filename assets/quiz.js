(function () {
  function initQuiz(quiz) {
    const feedback = quiz.querySelector(".feedback");
    const buttons = quiz.querySelectorAll("button[data-answer]");
    buttons.forEach((button) => {
      button.addEventListener("click", () => {
        const correct = button.dataset.answer === "true";
        const text = button.dataset.feedback || "";
        feedback.textContent = correct ? "Correct. " + text : "Not quite. " + text;
        feedback.style.borderLeftColor = correct ? "#0b6b5c" : "#8c2f39";
      });
    });
  }

  document.addEventListener("DOMContentLoaded", () => {
    document.querySelectorAll("[data-quiz]").forEach(initQuiz);
  });
})();
