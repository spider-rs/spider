#!/bin/sh
set -e

# ── Extract version from spider/Cargo.toml (the source of truth) ──
VERSION=$(grep '^version' spider/Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
if [ -z "$VERSION" ]; then
  echo "ERROR: could not read version from spider/Cargo.toml"
  exit 1
fi
echo "Publishing workspace at v${VERSION}"

# ── Sync internal cross-crate dependency versions ──
# spider_agent_html depends on spider_agent_types
sed -i.bak "s/spider_agent_types = { version = \"[^\"]*\"/spider_agent_types = { version = \"${VERSION}\"/" spider_agent_html/Cargo.toml
# spider_agent depends on spider_agent_types and spider_agent_html
sed -i.bak "s/spider_agent_types = { version = \"[^\"]*\"/spider_agent_types = { version = \"${VERSION}\"/" spider_agent/Cargo.toml
sed -i.bak "s/spider_agent_html = { version = \"[^\"]*\"/spider_agent_html = { version = \"${VERSION}\"/" spider_agent/Cargo.toml
# Clean up sed backup files
rm -f spider_agent_html/Cargo.toml.bak spider_agent/Cargo.toml.bak

echo "Internal deps synced to v${VERSION}"

# ── Commit + push the sync before publishing ──
# `cargo publish` (without --no-verify) refuses to package a crate whose
# Cargo.toml has uncommitted changes. Stage only the two files we just
# sed'd so unrelated working-tree changes don't get pulled into the sync
# commit.
if ! git diff --quiet -- spider_agent_html/Cargo.toml spider_agent/Cargo.toml; then
  git add spider_agent_html/Cargo.toml spider_agent/Cargo.toml
  git commit -m "chore(release): sync internal cross-crate deps to v${VERSION}"
  git push
fi

# ── Publish in dependency order ──
cd spider_agent_types && cargo publish; cd ../
cd spider_agent_html && cargo publish; cd ../
cd spider_agent && cargo publish; cd ../
cd spider && cargo publish --no-verify; cd ../
cd spider_cli && cargo publish --no-verify; cd ../
cd spider_utils && cargo publish --no-verify; cd ../
cd spider_worker && cargo publish --no-verify; cd ../
cd spider_mcp && cargo publish --no-verify; cd ../

git push
