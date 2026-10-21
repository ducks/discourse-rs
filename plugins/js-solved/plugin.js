// discourse-solved's has_accepted_answer, in JavaScript: a third-party
// plugin as a self-hoster would write it. rows() is synchronous, a call
// into the host the host answers before the script goes on.
const plugin = {
  topic_list: {
    after_serialize(topics) {
      const ids = topics.map((t) => t.id);
      const solved = rows("discourse_solved_solved_topics", "topic_id", ids);
      const answers = rows(
        "discourse_solved_topic_answers",
        "solved_topic_id",
        solved.map((s) => s.id),
      );
      const answered = new Set();
      for (const a of answers) {
        const s = solved.find((s) => s.id === a.solved_topic_id);
        if (s) answered.add(s.topic_id);
      }
      const out = {};
      for (const id of ids) out[id] = { has_accepted_answer: answered.has(id) };
      return out;
    },
  },
};
