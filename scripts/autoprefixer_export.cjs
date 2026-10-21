// node, on the reference agent: autoprefixer's prefixes for Discourse's
// browsers (frontend/discourse/config/targets.js), as AssetProcessor#post_css
// selects them, written as JSON for src/stylesheet/autoprefixer.rs. Reads
// the asset processor's own autoprefixer and caniuse-lite; changes nothing.
// scripts/record-autoprefixer-data runs it.
const { createRequire } = require("node:module");
const fs = require("node:fs");

const root = "/var/www/discourse/frontend/asset-processor/";
const autoprefixerDir = fs.realpathSync(root + "node_modules/autoprefixer");
const req = createRequire(autoprefixerDir + "/package.json");
const Browsers = req("./lib/browsers");
const Prefixes = req("./lib/prefixes");
const Declaration = req("./lib/declaration");
const agents = req("caniuse-lite/dist/unpacker/agents").agents;
const { browsers } = require("/var/www/discourse/frontend/discourse/config/targets.js");

const prefixes = new Prefixes(req("./data/prefixes"), new Browsers(agents, browsers, {}), {});
const regexp = (r) => [r.source, r.flags];
const oldValues = (side) => {
  const out = {};
  for (const [name, entry] of Object.entries(side)) {
    if (name === "selectors" || name[0] === "@" || !entry || !entry.values) continue;
    out[name] = entry.values
      .filter((o) => o.check)
      .map((o) => ({ unprefixed: o.unprefixed, string: o.string, regexp: regexp(o.regexp) }));
  }
  return out;
};
const removeProps = (side) =>
  Object.keys(side).filter((k) => k[0] !== "@" && side[k] && side[k].remove);

const normalize = {};
for (const name of Object.keys(Declaration.hacks)) {
  const normal = prefixes.normalize(name);
  if (normal !== name) normalize[name] = normal;
}

const addProps = {};
const addValues = {};
for (const [name, entry] of Object.entries(prefixes.add)) {
  if (name[0] === "@" || name === "selectors" || !entry) continue;
  if (entry.prefixes) addProps[name] = entry.prefixes;
  if (entry.values && entry.values.length) {
    addValues[name] = entry.values.map((v) => ({
      name: v.name,
      prefixes: v.prefixes,
      kind: v.constructor.name,
      regexp: regexp(v.regexp()),
    }));
  }
}

// Prefixes#prefixed for every property the adding side prefixes.
const prefixed = {};
for (const prop of new Set([...Object.keys(addProps), ...Object.keys(addValues)])) {
  for (const prefix of Browsers.prefixes()) {
    const name = prefixes.prefixed(prop, prefix);
    if (name !== prefix + prop) prefixed[prefix + prop] = name;
  }
}

const cleaner = prefixes.cleaner();
const resolution = prefixes.remove["@resolution"];
const supportsPrefixer = prefixes.add["@supports"].prefixer();
if (JSON.stringify(Object.keys(supportsPrefixer.add)) !== JSON.stringify(Object.keys(prefixes.add))) {
  throw new Error("@supports prefixes differ from the stylesheet's");
}
for (const at of Object.keys(prefixes.add)) {
  if (at[0] === "@" && at !== "@supports") throw new Error(`at-rule prefixes added: ${at}`);
}
if (prefixes.add.selectors.length) throw new Error("selector prefixes added");

const out = {
  autoprefixer: req("./package.json").version,
  browsers: browsers,
  prefixes: Browsers.prefixes(),
  normalize,
  prefixed,
  add_props: addProps,
  add_values: addValues,
  remove_props: removeProps(prefixes.remove),
  remove_values: oldValues(prefixes.remove),
  remove_selectors: prefixes.remove.selectors.map((s) => ({
    prefixed: s.prefixed,
    regexp: regexp(s.regexp),
    prefixeds: s.prefixeds.map(([name, r]) => [name, regexp(r)]),
    unprefixed: s.unprefixed,
    name_regexp: regexp(s.nameRegexp),
  })),
  remove_at_rules: Object.keys(prefixes.remove).filter((k) => k[0] === "@" && prefixes.remove[k].remove),
  resolution_prefixes: resolution ? resolution.prefixes : [],
  cleaner_remove_props: removeProps(cleaner.remove),
  cleaner_remove_values: oldValues(cleaner.remove),
};
process.stdout.write(JSON.stringify(out, null, 1) + "\n");
