#!/usr/bin/env bash
# Build the site and publish it. The pipeline does this on every push to master;
# this is the same steps by hand, for when you want to see it happen.
set -euo pipefail
cd "$(dirname "$0")/.."
STACK=seanshubin-com

# MSYS_NO_PATHCONV: see scripts/build-hotseat-web.sh for what it saves you from.
( cd apps/hotseat && MSYS_NO_PATHCONV=1 trunk build --release --public-url /liars-dice/hotseat/ )

rm -rf site
mkdir -p site/liars-dice/hotseat
cp web/index.html site/index.html
cp -r apps/hotseat/dist/* site/liars-dice/hotseat/

# Nothing published that cannot be reached from the root, nothing linked that was
# not published.
python scripts/check-site-links.py site

outputs() {
  aws cloudformation describe-stacks --stack-name "$STACK" \
    --query "Stacks[0].Outputs[?OutputKey=='$1'].OutputValue" --output text
}
bucket=$(outputs BucketName)
distribution=$(outputs DistributionId)

# Two passes: content-hashed assets can never change under their own URL, so a
# year is safe. HTML names them, so it must be re-read or a deploy publishes
# assets nothing asks for.
aws s3 sync site "s3://${bucket}" --delete --exclude "*.html" \
  --cache-control "public, max-age=31536000, immutable"
aws s3 sync site "s3://${bucket}" --exclude "*" --include "*.html" \
  --cache-control "no-cache"
aws cloudfront create-invalidation --distribution-id "${distribution}" --paths "/*" >/dev/null

echo "published to https://seanshubin.com/"
