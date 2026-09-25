# Evidence-linked structured notes

The MVP can produce a local extractive notes draft from the current corrected transcript. The schema covers overview passages, topics, decisions, actions, questions, risks, and next steps. Every generated item includes one or more source segment identifiers and timestamps; clicking a timestamp in the review dialog plays the corresponding local audio.

This first strategy is deliberately conservative. It selects transcript passages and does not paraphrase them, infer identity, or invent a deadline. Actions use `Not specified` for owner and due date until the transcript and a future reviewed local-model adapter provide explicit support. Empty categories remain visibly “Not identified in the transcript.”

The first generated draft is persisted locally. Users can edit every extracted passage plus action owners and due dates, save an unapproved draft, reopen it later, or deliberately regenerate it from the current corrected transcript. Editing or regeneration clears approval. Evidence identifiers and timestamps are immutable in the interface, and the backend rejects edited notes whose citations do not exactly match the meeting transcript.

A transcript correction marks only that meeting's notes stale and clears their approval without deleting manual edits. Stale drafts remain readable and editable, but both the interface and backend block approval/export until the user regenerates them from the corrected transcript.

Markdown and plain-text exports contain the exact reviewed draft, its evidence timestamps, and the extractive-draft warning. Export controls remain disabled until the user explicitly confirms that version; the backend independently rejects unconfirmed export and persists the approved version. Replaceable local generative-model output remains separate backlog work.
