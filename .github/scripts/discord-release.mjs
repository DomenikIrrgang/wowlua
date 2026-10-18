// Posts a release to the wowlua Discord's #releases and pings @Release pings: the version's CHANGELOG.md section,
// with the full notes a click away. release.yml runs it after the GitHub release is made. The release's own notes
// are generated from pull requests, so the post reads CHANGELOG.md instead.
//
// Env: TAG (v0.9.0), GITHUB_REPOSITORY, DISCORD_RELEASES_WEBHOOK (secret), DISCORD_RELEASE_ROLE_ID.
// `node .github/scripts/discord-release.mjs --dry-run` prints the message instead of posting it.

import { readFileSync } from "node:fs";

const { TAG, GITHUB_REPOSITORY = "DomenikIrrgang/wowlua", DISCORD_RELEASES_WEBHOOK, DISCORD_RELEASE_ROLE_ID } = process.env;
const dryRun = process.argv.includes("--dry-run");

if (!DISCORD_RELEASES_WEBHOOK && !dryRun) {
  console.log("No DISCORD_RELEASES_WEBHOOK: not posting to Discord.");
  process.exit(0);
}
if (!TAG) throw new Error("Set TAG, e.g. TAG=v0.9.0");

const version = TAG.replace(/^v/, "");
const releaseUrl = `https://github.com/${GITHUB_REPOSITORY}/releases/tag/${TAG}`;
const prerelease = version.includes("-");

// "## [0.9.0] - 2026-10-15", an intro, then ### Added / Changed / Fixed with one "- " line per entry.
const changelog = readFileSync(new URL("../../CHANGELOG.md", import.meta.url), "utf8").split(/\r?\n/);
// Without a section the post still goes out, with the link alone: the release exists by now either way.
const start = changelog.findIndex((l) => l.startsWith(`## [${version}]`));
if (start < 0) console.log(`CHANGELOG.md has no section for ${version}: posting the link alone.`);
const sections = {};
const intro = [];
let current = null;
for (const line of start < 0 ? [] : changelog.slice(start + 1)) {
  if (line.startsWith("## ")) break;
  const heading = line.match(/^###\s+(.*)/);
  if (heading) { current = heading[1].trim(); sections[current] = []; continue; }
  if (!current) { if (line.trim()) intro.push(line.trim()); continue; }
  if (line.startsWith("- ")) sections[current].push(line.slice(2).trim()); // top-level entries only
}

const short = (entry) => (entry.length > 160 ? `${entry.slice(0, 160).replace(/\s+\S*$/, "")}…` : entry);
const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;
const list = (title, entries) => `**${title}**\n${entries.map((e) => `• ${short(e)}`).join("\n")}`;
const added = sections.Added ?? [];
const changed = sections.Changed ?? [];
const fixed = sections.Fixed ?? [];
const link = `[All release notes](${releaseUrl})`;

// A minor release lists what's new and counts the rest; a patch release, with nothing new, lists what it changes.
const parts = [];
if (intro.length) parts.push(intro.join(" "));
if (added.length) {
  parts.push(list("New", added));
  const more = [changed.length && plural(changed.length, "change"), fixed.length && plural(fixed.length, "fix").replace("fixs", "fixes")].filter(Boolean);
  parts.push(more.length ? `**Also** ${more.join(" and ")}. ${link}` : link);
} else {
  if (changed.length) parts.push(list("Changed", changed));
  if (fixed.length) parts.push(list("Fixed", fixed));
  parts.push(link);
}
let description = parts.join("\n\n");
if (description.length > 4000) {
  // Discord's limit is 4096: keep the start of the list and the link.
  description = `${description.slice(0, 3800).replace(/\n[^\n]*$/, "")}\n…\n\n${link}`;
}

const role = DISCORD_RELEASE_ROLE_ID;
const message = {
  content: `${role ? `<@&${role}> ` : ""}**wowlua ${version}** is out.`,
  allowed_mentions: { roles: role ? [role] : [] },
  embeds: [{
    title: `wowlua ${version}`,
    url: releaseUrl,
    description,
    color: 0xffb547, // amber
    fields: [{
      name: "Update",
      value: `Run the [installer](https://wowlua.dev/docs/install#update-or-pin-a-version) again, or download it from [GitHub](${releaseUrl}).`,
    }],
    footer: { text: prerelease ? "Pre-release: things can still change" : "Release" },
    timestamp: new Date().toISOString(),
  }],
};

if (dryRun) {
  console.log(JSON.stringify(message, null, 2));
} else {
  const post = await fetch(`${DISCORD_RELEASES_WEBHOOK}?wait=true`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(message),
  });
  if (!post.ok) throw new Error(`Discord: ${post.status} ${await post.text()}`);
  console.log(`Posted wowlua ${version} to Discord.`);
}
