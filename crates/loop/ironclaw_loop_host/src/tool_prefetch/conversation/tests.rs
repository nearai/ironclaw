//! Building and segmenting the conversation context: its bounds (message
//! count, per-message cap, hard ceiling, segment size and count) and a
//! character-boundary-safe split.

use super::*;

#[test]
fn conversation_context_caps_the_text_and_never_prints_it() {
    let long = format!("  {}é  ", "a".repeat(MAX_CONTEXT_MESSAGE_BYTES - 1));
    let context = opening_request(&long);
    assert_eq!(context.user_messages().len(), 1);
    // The two-byte character straddles the cap, so the cut lands before it.
    assert_eq!(
        context.user_messages()[0].len(),
        MAX_CONTEXT_MESSAGE_BYTES - 1
    );
    assert!(!context.is_empty());
    assert!(opening_request("   ").is_empty());
    let printed = format!("{:?}", opening_request("private words"));
    assert!(!printed.contains("private"), "{printed}");
    assert!(printed.contains("bytes"));
}

#[test]
fn window_keeps_the_newest_messages_then_the_first_if_it_fits() {
    // Newest first: "c" is the latest message. Blank messages do not count.
    let context = window(["cccc", "  ", "bbbb", "aaaa"], Some("f"), 3);
    assert_eq!(context.user_messages(), ["aaaa", "bbbb", "cccc"]);

    // The first message fits while the count leaves room for it.
    let context = window(["cccc", "bbbb"], Some(" first "), 3);
    assert_eq!(context.user_messages(), ["first", "bbbb", "cccc"]);

    // The count is at least one.
    let context = window(["cccc", "bbbb"], Some("first"), 0);
    assert_eq!(context.user_messages(), ["cccc"]);

    // Every message is cut to the per-message cap; the window stops where
    // the hard ceiling would be passed, and the first message is left out
    // when it no longer fits under it.
    let huge = "x".repeat(MAX_CONTEXT_MESSAGE_BYTES * 2);
    let context = window(
        [huge.as_str(), huge.as_str(), huge.as_str()],
        Some("first"),
        MAX_CONTEXT_MESSAGES,
    );
    assert_eq!(context.user_messages().len(), 2);
    let total: usize = context.user_messages().iter().map(String::len).sum();
    assert_eq!(total, MAX_CONVERSATION_CONTEXT_BYTES);
    assert!(
        context
            .user_messages()
            .iter()
            .all(|message| message.len() == MAX_CONTEXT_MESSAGE_BYTES)
    );
    assert!(!format!("{context:?}").contains("xxx"));

    // The count never exceeds the hard maximum.
    let many: Vec<String> = (0..MAX_CONTEXT_MESSAGES * 2)
        .map(|index| format!("m{index}"))
        .collect();
    let context = window(many.iter().map(String::as_str), None, usize::MAX);
    assert_eq!(context.user_messages().len(), MAX_CONTEXT_MESSAGES);
    assert_eq!(
        context.user_messages().last().map(String::as_str),
        Some("m0")
    );
}

#[test]
fn segments_run_newest_message_first_and_keep_short_messages_whole() {
    let context = window(
        ["newest words", "middle words", "oldest words"],
        Some("first words"),
        16,
    );
    assert_eq!(
        segments(&context, 2_048, 16),
        [
            "newest words",
            "middle words",
            "oldest words",
            "first words"
        ]
    );
    // The count bounds the segments, newest kept.
    assert_eq!(
        segments(&context, 2_048, 2),
        ["newest words", "middle words"]
    );
    assert_eq!(segments(&context, 2_048, 0), ["newest words"]);
    assert!(segments(&opening_request("  "), 2_048, 16).is_empty());
}

#[test]
fn a_long_message_is_cut_at_paragraphs_then_sentences_then_words() {
    let paragraph = |word: &str| {
        let mut text = String::new();
        for sentence in 0..6 {
            text.push_str(&format!("{word} sentence number {sentence} is here. "));
        }
        text.trim_end().to_string()
    };
    // Three paragraphs of about 200 bytes each; a 450-byte budget holds two.
    let message = format!(
        "{}\n\n{}\n\n{}",
        paragraph("alpha"),
        paragraph("beta"),
        paragraph("gamma")
    );
    let context = opening_request(&message);
    let cut = segments(&context, 450, 16);
    assert_eq!(cut.len(), 2, "{cut:?}");
    assert!(cut[0].starts_with("alpha") && cut[0].ends_with("is here."));
    assert!(cut[0].contains("beta"));
    assert_eq!(cut[1], paragraph("gamma"));

    // No paragraph break in reach: the cut falls after a sentence.
    let one_paragraph = paragraph("delta").repeat(3);
    let context = opening_request(&one_paragraph);
    let cut = segments(&context, 128, 64);
    assert!(cut.len() > 3);
    for segment in &cut[..cut.len() - 1] {
        assert!(segment.ends_with('.'), "{segment:?}");
    }

    // No sentence end either: the cut falls between words.
    let words = "word ".repeat(200);
    let context = opening_request(&words);
    let cut = segments(&context, 128, 64);
    assert!(cut.iter().all(|segment| segment.len() <= 128));
    assert!(
        cut.iter()
            .all(|segment| segment.split(' ').all(|word| word == "word"))
    );
    assert_eq!(
        cut.iter()
            .map(|segment| segment.split(' ').count())
            .sum::<usize>(),
        200,
        "no word is lost or split"
    );
}

#[test]
fn segmentation_bounds_hold_and_never_split_a_character() {
    // One unbroken run of three-byte characters: every cut is a hard cut.
    let text = "日".repeat(MAX_CONTEXT_MESSAGE_BYTES);
    let context = opening_request(&text);
    let cut = segments(&context, 1_000, MAX_CONTEXT_MESSAGES);
    assert!(!cut.is_empty());
    for segment in &cut {
        assert!(segment.len() <= 1_000);
        assert!(segment.chars().all(|character| character == '日'));
    }
    assert_eq!(
        cut.iter().map(|segment| segment.len()).sum::<usize>(),
        context.user_messages()[0].len(),
        "the segments cover the message"
    );

    // The segment size is held between its bounds.
    let long = "abcdefgh ".repeat(2_000);
    let context = opening_request(&long);
    assert!(
        segments(&context, 1, 64)
            .iter()
            .all(|segment| segment.len() <= MIN_CONTEXT_SEGMENT_BYTES
                && segment.len() > MIN_CONTEXT_SEGMENT_BYTES / 2)
    );
    let largest = segments(&context, usize::MAX, 64);
    assert!(
        largest
            .iter()
            .all(|segment| segment.len() <= MAX_CONTEXT_SEGMENT_BYTES)
    );
    assert!(largest[0].len() > MAX_CONTEXT_SEGMENT_BYTES / 2);
    // And so is the count.
    assert_eq!(
        segments(&context, 128, usize::MAX).len(),
        MAX_CONTEXT_MESSAGES
    );
}
