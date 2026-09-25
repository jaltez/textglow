use crate::llm::ChatMsg;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    GlowUp,
    Professional,
    Casual,
    Shorter,
    Longer,
    FixGrammar,
}

impl Tone {
    pub const ALL: [Tone; 6] = [
        Tone::GlowUp,
        Tone::Professional,
        Tone::Casual,
        Tone::Shorter,
        Tone::Longer,
        Tone::FixGrammar,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Tone::GlowUp => "Glow up",
            Tone::Professional => "Professional",
            Tone::Casual => "Casual",
            Tone::Shorter => "Shorter",
            Tone::Longer => "Longer",
            Tone::FixGrammar => "Fix grammar",
        }
    }

    pub fn instruction(&self) -> &'static str {
        match self {
            Tone::GlowUp => {
                "Elevate and polish this text. Fix grammar and punctuation, improve the flow, \
                 and replace awkward phrasing with clear, engaging vocabulary. The text should \
                 read beautifully and naturally while preserving the original meaning and approximate length."
            }
            Tone::Professional => {
                "Rewrite this text for a professional workplace context. Make it clear, objective, \
                 and polite. Remove emotional language and slang. Ensure it sounds confident \
                 and competent without being robotic, overly stiff, or bureaucratic."
            }
            Tone::Casual => {
                "Rewrite this text to sound casual, friendly, and conversational—like a natural \
                 message to a colleague or friend. Use everyday language but avoid excessive \
                 slang or adding emojis unless they match the original text."
            }
            Tone::Shorter => {
                "Condense this text to make it significantly shorter and punchier. Strip out fluff, \
                 redundancy, and filler words. Use clear, direct sentences while retaining all \
                 crucial information and key points."
            }
            Tone::Longer => {
                "Expand this text to add depth, nuance, and smoother transitions, making it roughly \
                 1.5 to 2 times its current length. Elaborate logically on the existing points. \
                 CRITICAL: Do not invent new facts or hallucinate details not implied by the original text."
            }
            Tone::FixGrammar => {
                "Correct ONLY the grammar, spelling, and punctuation errors in this text. Make the \
                 absolute minimal changes required for correctness. Do not paraphrase, reword, \
                 or attempt to improve the style. Keep the original phrasing exactly as it is."
            }
        }
    }
}

pub const DEFAULT_SYSTEM_PROMPT: &str = "\
You are an expert text-rewriting assistant. You will receive instructions and text delimited by <text></text> tags.

CRITICAL CONSTRAINTS:
1. Return ONLY the final rewritten text.
2. Do NOT include preambles, acknowledgments, or explanations.
3. Do NOT wrap the output in quotation marks or markdown code fences.
4. Preserve the original language, formatting (lists, line breaks), and core meaning unless explicitly instructed otherwise.";

/// The override if it is non-blank, otherwise the built-in prompt.
pub fn system_prompt(custom: &str) -> &str {
    let t = custom.trim();
    if t.is_empty() {
        DEFAULT_SYSTEM_PROMPT
    } else {
        t
    }
}

/// First-round messages: system prompt + user message combining tone, optional
/// extra instructions, and the text to rewrite.
pub fn build_first_messages(
    system: &str,
    text: &str,
    tone: Tone,
    extra_instructions: Option<&str>,
) -> Vec<ChatMsg> {
    let mut user = String::new();
    user.push_str(tone.instruction());
    if let Some(extra) = extra_instructions.map(str::trim).filter(|e| !e.is_empty()) {
        user.push_str("\n\nAdditionally, follow these specific instructions: ");
        user.push_str(extra);
    }
    user.push_str("\n\n<text>\n");
    user.push_str(text);
    user.push_str("\n</text>");

    vec![ChatMsg::system(system), ChatMsg::user(user)]
}

/// A follow-up instruction applied to the previous result (refine round).
pub fn refine_message(instruction: &str) -> ChatMsg {
    ChatMsg::user(instruction.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tones_have_distinct_labels_and_instructions() {
        for t in Tone::ALL {
            assert!(!t.label().is_empty());
            assert!(t.instruction().len() > 20, "{:?} too short", t);
        }
    }

    #[test]
    fn first_messages_compose_tone_extra_and_text() {
        let msgs = build_first_messages(
            "SYS",
            "hello world",
            Tone::Professional,
            Some("add a greeting"),
        );
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[0].content, "SYS");
        let user = &msgs[1].content;
        assert!(user.starts_with(Tone::Professional.instruction()));
        let tone_pos = user.find(Tone::Professional.instruction()).unwrap();
        let extra_pos = user
            .find("Additionally, follow these specific instructions: add a greeting")
            .unwrap();
        let text_pos = user.find("<text>\nhello world\n</text>").unwrap();
        assert!(tone_pos < extra_pos && extra_pos < text_pos);
    }

    #[test]
    fn blank_extra_instructions_are_omitted() {
        let msgs = build_first_messages("SYS", "t", Tone::GlowUp, Some("   "));
        assert!(!msgs[1].content.contains("Additionally"));
    }

    #[test]
    fn system_prompt_override() {
        assert_eq!(system_prompt("  "), DEFAULT_SYSTEM_PROMPT);
        assert_eq!(system_prompt(" be fancy "), "be fancy");
    }
}
