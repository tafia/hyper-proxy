> Legend:
  - feat: A new feature
  - fix: A bug fix
  - docs: Documentation only changes
  - style: White-space, formatting, missing semi-colons, etc
  - refactor: A code change that neither fixes a bug nor adds a feature
  - perf: A code change that improves performance
  - test: Adding missing tests
  - chore: Changes to the build process or auxiliary tools/libraries/documentation

## 0.2.0
- feat: Add Intercept::None to never intercept any connection
- fix: Add Send + Sync constraints on Intercept::Custom function (breaking)
- feat: Make Intercept::matches function public
- feat: Add several function to get/modify internal states
