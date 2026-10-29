#!/usr/bin/env bash
# Build the site into docs/book: the landing page (docs/site) at /, the mdbook at /docs/, a sitemap, and stubs that
# send the book's old URLs (/<page>.html, before it moved to /docs/) on to their new place.
set -euo pipefail

cd "$(dirname "$0")"
site_url=https://lightmime.pataar.nl

rm -rf book
mdbook build
cp -r site/. book/
# GitHub Pages serves only the root 404.html; the book's version loads its assets from /docs/.
cp book/docs/404.html book/
# mdbook fingerprints its copy of the favicon; the landing page needs a stable URL.
cp theme/favicon.svg book/

pages=$(cd book/docs && ls -- *.html | grep -vx -e 404.html -e index.html -e print.html -e toc.html)

for page in $pages; do
    cat >"book/$page" <<EOF
<!doctype html>
<html lang="en">
<meta charset="utf-8">
<title>Moved to /docs/$page</title>
<meta name="robots" content="noindex">
<link rel="canonical" href="$site_url/docs/$page">
<meta http-equiv="refresh" content="0; url=/docs/$page">
<script>location.replace("/docs/$page" + location.hash)</script>
<p>This page moved to <a href="/docs/$page">/docs/$page</a>.</p>
</html>
EOF
done

{
    echo '<?xml version="1.0" encoding="UTF-8"?>'
    echo '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">'
    echo "  <url><loc>$site_url/</loc></url>"
    echo "  <url><loc>$site_url/docs/</loc></url>"
    for page in $pages; do
        echo "  <url><loc>$site_url/docs/$page</loc></url>"
    done
    echo '</urlset>'
} >book/sitemap.xml
