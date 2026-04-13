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
