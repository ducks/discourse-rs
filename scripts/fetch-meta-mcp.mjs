#!/usr/bin/env node
// Anonymous, read-only MCP sampling. Corpus files stay in ignored backups/.
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { resolve, join } from "node:path";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { setTimeout as sleep } from "node:timers/promises";

const [mcpRoot, destination] = process.argv.slice(2);
if (!mcpRoot || !destination) throw new Error("usage: node scripts/fetch-meta-mcp.mjs <built-discourse-mcp-checkout> <output-directory>");
const root = resolve(mcpRoot);
const out = resolve(destination);
const require = createRequire(join(root, "package.json"));
const { Client } = await import(pathToFileURL(require.resolve("@modelcontextprotocol/sdk/client/index.js")));
const { StdioClientTransport } = await import(pathToFileURL(require.resolve("@modelcontextprotocol/sdk/client/stdio.js")));
const site = "https://meta.discourse.org";
const client = new Client({ name: "discourse-rs-public-corpus", version: "1.0.0" });
const transport = new StdioClientTransport({
  command: process.execPath,
  args: [join(root, "dist/index.js"), "--site", site, "--auth_pairs", "[]",
    "--read_only", "true", "--tools_mode", "discourse_api_only",
    "--toolsets", "topics,search,administration", "--concurrency", "1",
    "--max-read-length", "200000", "--log_level", "error"],
  stderr: "inherit",
});
await mkdir(join(out, "topics"), { recursive: true });
const failures = [];
async function call(name, args) {
  await sleep(1500);
  const response = await client.callTool({ name, arguments: args }, undefined, { timeout: 120000 });
  if (response.isError) throw new Error(JSON.stringify(response.content));
  return response.structuredContent ?? JSON.parse(response.content.find(c => c.type === "text").text);
}
async function save(name, data) {
  await writeFile(join(out, name), JSON.stringify(data, null, 2) + "\n");
}
try {
  await client.connect(transport);
  const categories = await call("discourse_list_categories", { max_pages: 3, max_requests: 6, max_results: 100 });
  await save("categories.json", categories);
  const candidates = new Map();
  async function discover(label, name, args, take) {
    try {
      const data = await call(name, args);
      await save(`discovery-${label}.json`, data);
      for (const topic of (data.results ?? []).slice(0, take)) {
        const existing = candidates.get(topic.id);
        if (existing) existing.strata.push(label);
        else candidates.set(topic.id, { ...topic, strata: [label] });
      }
    } catch (error) {
      failures.push({ stage: label, error: String(error) });
      console.error(`${label}: ${error}`);
    }
  }
  await discover("top-all", "discourse_filter_topics", { view: "top", top_period: "all", per_page: 10 }, 8);
  await discover("top-year", "discourse_filter_topics", { view: "top", top_period: "yearly", per_page: 10 }, 6);
  for (const [label, query] of [
    ["recent", "after:2026-01-01 order:latest"],
    ["older", "before:2020-01-01 order:likes"],
    ["long", "min_posts:100 order:latest"],
    ["short", "max_posts:3 order:latest"],
    ["support", "category:support order:latest"],
    ["bug", "category:bug order:latest"],
    ["dev", "category:dev order:latest"],
    ["feature", "category:feature order:latest"],
    ["howto", "category:howto order:likes"],
  ]) await discover(label, "discourse_search", { query, max_results: 5 }, 5);
  await save("selection.json", [...candidates.values()]);
  const topics = [];
  for (const candidate of candidates.values()) {
    const filename = `topics/${candidate.id}.json`;
    try {
      let topic;
      try { topic = JSON.parse(await readFile(join(out, filename), "utf8")); }
      catch (error) { if (error.code !== "ENOENT") throw error; }
      if (topic?.sample_version !== 2) topic = undefined;
      if (!topic) {
        const metadata = await call("discourse_read_topic", { topic_id: candidate.id, post_limit: 20 });
        const ids = [...new Set(metadata.posts.map(p => p.id))];
        if (!ids.length) throw new Error("no public posts");
        // The post_ids stream can contain duplicate IDs. Select the opening
        // window from the topic response, then fetch exact IDs for raw/type flags.
        const first = await call("discourse_read_topic_posts", { topic_id: candidate.id, selection_mode: "post_ids", post_ids: ids });
        const batches = [first];
        if (metadata.posts_count > 20) batches.push(await call("discourse_read_topic_posts", { topic_id: candidate.id, selection_mode: "latest", limit: 50 }));
        const posts = [...new Map(batches.flatMap(b => b.posts).map(p => [p.id, p])).values()]
          .filter(p => p.post_type === 1 && !p.hidden && !p.deleted_at && typeof p.raw === "string" && p.raw.length && !p.truncated)
          .sort((a, b) => a.post_number - b.post_number);
        if (posts[0]?.post_number !== 1) throw new Error("no complete public opening post");
        const { posts: ignoredPosts, ...details } = metadata;
        topic = { ...details, sample_version: 2, source: `${site}/t/${candidate.id}`, strata: candidate.strata,
          access: "anonymous", sampled_at: new Date().toISOString(),
          posts };
        await save(filename, topic);
      }
      topics.push(topic);
      console.log(`${topics.length}/${candidates.size}: topic ${topic.id}, ${topic.posts.length}/${topic.posts_count} sampled posts`);
    } catch (error) {
      failures.push({ topic_id: candidate.id, error: String(error) });
      console.error(`topic ${candidate.id}: ${error}`);
    }
  }
  const corpus = { version: 1, site, access: "anonymous", transport: "mcp-stdio", categories: categories.categories, topics, failures };
  await save("corpus.json", corpus);
  const posts = topics.flatMap(t => t.posts);
  await save("summary.json", { topics: topics.length, posts: posts.length,
    authors: new Set(posts.map(p => p.username)).size,
    dates: posts.map(p => p.created_at).sort().filter((_, i, a) => i === 0 || i === a.length - 1),
    content_features: Object.fromEntries(Object.entries({ code_fence: /```/, quote: /\[quote[=\]]/i,
      image: /!\[|<img\b/i, details: /\[details/i, poll: /\[poll/i, non_ascii: /[^\x00-\x7f]/,
    }).map(([name, regex]) => [name, posts.filter(p => regex.test(p.raw)).length])),
    categories: [...new Set(topics.map(t => t.category_id))], failures });
  if (!topics.length) throw new Error("no public topics collected");
  console.log(`Saved ${topics.length} topics to ${out}/corpus.json (${failures.length} failures)`);
} finally { await client.close(); }
