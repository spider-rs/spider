#!/bin/sh

cd spider_agent_types && cargo publish && cd ../
cd spider_agent_html && cargo publish && cd ../
cd spider_agent && cargo publish && cd ../
cd spider && cargo publish && cd ../
cd spider_cli && cargo publish --no-verify && cd ../
cd spider_utils && cargo publish --no-verify && cd ../
cd spider_worker && cargo publish --no-verify && cd ../
cd spider_mcp && cargo publish --no-verify && cd ../
