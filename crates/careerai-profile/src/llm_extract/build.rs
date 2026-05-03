use super::types::{ExtractOptions, ExtractRequest};

/// Build the system message. Static so it's easy to snapshot.
#[must_use]
pub fn build_system_message() -> String {
    "You are a resume parser. Extract structured data from the resume \
text the user provides. You MUST output a single JSON object that \
matches the supplied schema, with no prose, no markdown fences, and no \
trailing commentary. Never invent experience, titles, dates, or \
employers — if a field is unknown, use an empty string for scalars and \
an empty array for lists. Dates must be `YYYY-MM`, `YYYY`, the literal \
`Present`, or empty."
        .to_string()
}

/// The schema preamble sent in the cacheable profile_block.
#[must_use]
pub fn build_schema_preamble() -> String {
    r#"OUTPUT SCHEMA (JSON):
{
  "personal": {
    "name": "string (required)",
    "email": "string",
    "phone": "string",
    "location": "string",
    "links": {
      "github": "string (URL)",
      "linkedin": "string (URL)",
      "portfolio": "string (URL)"
    }
  },
  "summary": "string",
  "skills": {
    "languages": ["string", ...],
    "frameworks": ["string", ...],
    "tools": ["string", ...]
  },
  "experience": [
    {
      "title": "string",
      "company": "string",
      "location": "string",
      "start": "YYYY-MM | YYYY | empty",
      "end":   "YYYY-MM | YYYY | Present | empty",
      "bullets": ["string", ...]
    }
  ],
  "education": [
    {
      "degree": "string",
      "institution": "string",
      "start": "YYYY | YYYY-MM | empty",
      "end":   "YYYY | YYYY-MM | empty"
    }
  ],
  "projects": [
    {
      "name": "string",
      "url": "string (URL or empty)",
      "bullets": ["string", ...]
    }
  ]
}

RULES:
- Output a single JSON object matching the schema. No prose, no markdown.
- Never invent. Use "" for unknown scalars, [] for unknown lists.
- Each experience entry's `title` must be a job title, not a date.
  If the resume only shows a date and no title, leave `title` empty.
- Each education entry's `institution` must be the school name; the
  `degree` is the qualification (e.g. "B.Tech Computer Science").
- Bullets are individual achievement lines; do not concatenate.
"#
    .to_string()
}

/// Build the user message wrapping the resume text.
#[must_use]
pub fn build_user_message(resume_text: &str) -> String {
    let trimmed = resume_text.trim();
    format!("RESUME TEXT (verbatim, may contain OCR artifacts):\n---\n{trimmed}\n---\n\nReturn the JSON object now.")
}

/// Build the full extraction request. Pure function — easy to snapshot.
#[must_use]
pub fn build_request(resume_text: &str, opts: &ExtractOptions) -> ExtractRequest {
    ExtractRequest {
        system: build_system_message(),
        profile_block: build_schema_preamble(),
        user: build_user_message(resume_text),
        prompt_version: opts.prompt_version.clone(),
        model: opts.model.clone(),
        temperature: opts.temperature,
        max_tokens: opts.max_tokens,
        cache_schema: opts.cache_schema,
    }
}
