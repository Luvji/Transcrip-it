# Evidence-linked structured notes

The MVP can produce a local extractive notes draft from the current corrected transcript. The schema covers overview passages, topics, decisions, actions, questions, risks, and next steps. Every generated item includes one or more source segment identifiers and timestamps; clicking a timestamp in the review dialog plays the corresponding local audio.

This first strategy is deliberately conservative. It selects transcript passages and does not paraphrase them, infer identity, or invent a deadline. Actions use `Not specified` for owner and due date until the transcript and a future reviewed local-model adapter provide explicit support. Empty categories remain visibly “Not identified in the transcript.”

Notes are generated from the current transcript on demand, so user corrections are reflected the next time the dialog opens. Markdown and plain-text exports include evidence timestamps and the extractive-draft warning. Transcript and notes export controls remain disabled until the user explicitly confirms review, and backend commands reject an unconfirmed export. Editable notes and replaceable local generative-model output remain separate backlog work.
