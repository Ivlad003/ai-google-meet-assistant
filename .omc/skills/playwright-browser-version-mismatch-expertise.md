---
name: playwright-browser-version-mismatch
description: Playwright browsers must be installed AFTER copying node_modules to match the npm package version
triggers:
  - "Executable doesn't exist at /ms-playwright"
  - "Please update docker image"
  - "playwright install chromium Docker"
  - "chrome-linux/chrome not found"
---

# Playwright Browser Version Mismatch in Docker

## The Insight
`npx playwright install chromium` installs browsers matching whichever playwright npm package is in the current node_modules. If you run this BEFORE copying your app's node_modules, it uses the base image's global playwright version — which may differ from your app's pinned version by a patch release.

## Why This Matters
Playwright 1.56.0 and 1.56.1 install browsers to DIFFERENT paths (`chromium-1193` vs `chromium-1194`). The npm package hardcodes the expected path. A 0.0.1 mismatch = "Executable doesn't exist" crash.

## Recognition Pattern
- Dockerfile installs playwright browsers in one layer, copies node_modules in a later layer
- Error: `browserType.launch: Executable doesn't exist at /ms-playwright/chromium-XXXX/chrome-linux/chrome`
- The error message helpfully tells you current vs required image version

## The Approach
Always install Playwright browsers AFTER copying your app's node_modules:

```dockerfile
# WRONG ORDER:
RUN npx playwright install chromium    # Uses base image's playwright version
COPY --from=builder /app/node_modules  # App may have different version

# RIGHT ORDER:
COPY --from=builder /app/node_modules  # Copy app's playwright version first
RUN cd /app && npx playwright install --with-deps chromium  # Matches npm package
```
