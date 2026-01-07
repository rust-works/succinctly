# Pull Request Guidelines

This document provides comprehensive guidelines for writing effective pull request descriptions that facilitate code review, ensure project quality, and maintain clear project history.

## Table of Contents

1. [Overview](#overview)
2. [AI-Generated PR Descriptions](#ai-generated-pr-descriptions)
3. [PR Title Guidelines](#pr-title-guidelines)
4. [Description Best Practices](#description-best-practices)
5. [Template Section Guidelines](#template-section-guidelines)
6. [Code Review Facilitation](#code-review-facilitation)
7. [Common Patterns](#common-patterns)
8. [Examples](#examples)

## Overview

A well-written pull request serves multiple purposes:

- **Documentation**: Explains what changed and why
- **Review Aid**: Helps reviewers understand context and focus areas
- **Project History**: Creates searchable record for future reference
- **Quality Assurance**: Ensures all necessary steps are completed

### Key Principles

1. **Clarity**: Write for reviewers who may not have context
2. **Completeness**: Include all necessary information for review
3. **Conciseness**: Be thorough but avoid unnecessary verbosity
4. **Actionability**: Provide clear testing and validation steps

## AI-Generated PR Descriptions

**CRITICAL FOR AI ASSISTANTS**: When generating PR descriptions, you MUST:

1. **ALWAYS REPLACE PLACEHOLDERS**: Never leave placeholder text like "TODO", "FIXME", "Add description here", "<!--comment-->", or generic instructions in brackets
2. **FILL IN ACTUAL CONTENT**: Every section must contain specific, concrete information based on the actual code changes and diffs
3. **BE SPECIFIC ABOUT CHANGES**: List the exact files modified, functions added, and features implemented
4. **NO GENERIC TEXT**: Avoid generic statements like "various improvements" - specify what was improved and how
5. **REMOVE TEMPLATE COMMENTS**: Delete all HTML comments and instructional text from the template
6. **ANALYZE THE DIFFS**: Read through all commit diffs to understand what actually changed
7. **PROVIDE REAL EXAMPLES**: If the template asks for test commands, provide the actual commands that should be run
8. **COMPLETE ALL CHECKLISTS**: Mark checklist items as checked `[x]` only if they were actually completed based on the commits

### What NOT to Do

❌ **NEVER do this:**
```markdown
## Description
<!-- Provide a clear, comprehensive overview of changes -->
Add description here.

## Changes Made
**Core Changes:**
-
-

**Documentation:**
-
```

✅ **ALWAYS do this instead:**
```markdown
## Description
This PR adds draft PR support with `--draft` and `--ready` CLI flags. The default behavior creates draft PRs, but users can override this via command-line flags or the `OMNI_DEV_DEFAULT_DRAFT_PR` configuration setting.

## Changes Made
**Core Changes:**
- Added `--draft` and `--ready` flags to `CreatePrCommand` in `src/cli/git.rs`
- Implemented `should_create_as_draft()` method with priority-based configuration resolution
- Updated `create_github_pr()` to pass `--draft` flag to GitHub CLI when appropriate

**Documentation:**
- Updated help text to document new flags
- Added configuration example to README
```

### AI Generation Checklist

Before returning a PR description, verify:
- [ ] All HTML comments and instructional text removed
- [ ] All placeholder bullet points replaced with actual content
- [ ] All "TODO" and "FIXME" markers removed
- [ ] Specific file names and functions mentioned where relevant
- [ ] Test commands are actual commands, not placeholders
- [ ] Checklist items accurately reflect what was done
- [ ] No sections left empty (either fill them or remove them)
- [ ] Description explains WHAT changed and WHY based on actual commits

## PR Title Guidelines

### Format
Use conventional commit format for consistency:
```
<type>(<scope>): <description>
```

### Types
- `feat`: New features or enhancements
- `fix`: Bug fixes
- `docs`: Documentation changes
- `refactor`: Code restructuring without functional changes
- `test`: Adding or updating tests
- `chore`: Maintenance tasks, dependency updates
- `perf`: Performance improvements
- `ci`: CI/CD pipeline changes

### Scope Examples
- `auth`: Authentication systems
- `api`: REST API changes
- `ui`: User interface components
- `cli`: Command-line interface
- `core`: Core application logic
- `docs`: Documentation system

### Title Best Practices

✅ **Good Examples:**
- `feat(auth): implement OAuth2 Google integration`
- `fix(api): resolve rate limiting edge case in user endpoints`
- `docs(readme): update installation instructions for macOS`
- `refactor(core): extract common validation logic`

❌ **Avoid:**
- `Update stuff` (too vague)
- `Fix bug` (no context)
- `Add new feature for users` (too verbose)
- `WIP: working on auth` (should not be merged)

## Description Best Practices

### Structure
1. **What**: Clear summary of changes
2. **Why**: Motivation and context
3. **How**: Implementation approach (if complex)
4. **Impact**: User-facing or architectural implications

### Writing Style
- Use present tense ("Add feature" not "Added feature")
- Be specific about changes made
- Explain non-obvious decisions
- Link to relevant issues or discussions

### Technical Details
- Include architectural decisions and trade-offs
- Mention performance implications
- Note any breaking changes prominently
- Reference related documentation

## Template Section Guidelines

### Description Section
**Purpose**: Provide clear, comprehensive overview of changes

**Best Practices:**
- Start with one-sentence summary
- Explain the problem being solved
- Describe the solution approach
- Mention user-facing impact

**Example:**
```markdown
## Description

This PR implements OAuth2 authentication for Google and GitHub providers, enabling users to sign in without creating separate accounts. The implementation follows OAuth2 best practices and includes comprehensive error handling for edge cases like expired tokens and network failures.

The changes include a new authentication service, middleware for token validation, and updated UI components for the login flow.
```

### Type of Change Section
**Purpose**: Categorize the PR for reviewers and release notes

**Guidelines:**
- Select all applicable types
- Use "Breaking change" sparingly and document migration
- Choose "Bug fix" only for actual defects
- "Refactoring" should have no functional changes

### Related Issue Section
**Purpose**: Link PR to project tracking and context

**Best Practices:**
- Always link to issues when available
- Use "Fixes #123" for issues this PR closes
- Use "Relates to #123" for partial progress
- Include links to discussions or design docs

### Changes Made Section
**Purpose**: Provide detailed list of modifications

**Guidelines:**
- Be specific about what was changed
- Group related changes together
- Use consistent formatting
- Focus on significant changes, not trivial ones

**Example:**
```markdown
## Changes Made

**Authentication System:**
- Added OAuth2 service with Google and GitHub providers
- Implemented JWT token validation and refresh logic
- Created secure token storage with encryption

**User Interface:**
- Updated login page with OAuth2 buttons
- Added account linking flow for existing users
- Improved error messaging for auth failures

**Testing & Documentation:**
- Added comprehensive test suite for auth flows
- Updated API documentation with new endpoints
- Created user guide for OAuth2 setup
```

### Testing Section
**Purpose**: Demonstrate quality assurance and provide validation steps

**Guidelines:**
- Include both automated and manual testing
- Provide specific reproduction steps
- Mention edge cases tested
- Include performance testing if relevant

**Example:**
```markdown
## Testing

**Automated Tests:**
- [x] All existing tests pass
- [x] New tests added for OAuth2 flows (95% coverage)
- [x] Integration tests for both Google and GitHub providers

**Manual Testing:**
- [x] Tested complete sign-up flow with Google OAuth2
- [x] Verified account linking for existing users
- [x] Tested error scenarios (network failures, invalid tokens)
- [x] Confirmed token refresh works correctly

**Edge Cases Tested:**
- User cancels OAuth2 flow midway
- Provider returns unexpected error responses
- Network timeouts during token exchange
- Concurrent login attempts from same user
```

### Checklist Section
**Purpose**: Ensure code quality and completeness

**Guidelines:**
- Only check items that are actually complete
- Add project-specific items as needed
- Use as final quality gate before review
- Don't skip items that don't apply - explain why

### Breaking Changes Section
**Purpose**: Highlight backward compatibility issues

**Guidelines:**
- Only use for actual breaking changes
- Provide clear migration instructions
- Include version information if applicable
- Link to migration guides

**Example:**
```markdown
## Breaking Changes

**API Changes:**
- The `/auth` endpoint now requires `provider` parameter
- Legacy password authentication deprecated (will be removed in v2.0)

**Migration Required:**
1. Update API calls to include `provider: "local"` for existing users
2. Replace direct password validation with new auth service
3. Update environment variables (see [Migration Guide](docs/migration-v1.5.md))

**Backward Compatibility:**
- Legacy endpoints remain functional until v2.0
- Existing user accounts are automatically migrated
- No changes required for end users
```

## Code Review Facilitation

### Highlighting Review Areas
Guide reviewers to focus areas:

```markdown
## Review Focus Areas

**Security Considerations:**
- OAuth2 token handling in `src/auth/oauth.rs`
- Secure cookie configuration in `src/middleware/auth.rs`

**Performance Impact:**
- New database queries in user authentication flow
- Token validation caching strategy

**Architecture Changes:**
- New authentication service interface
- Modified user model to support multiple providers
```

### Self-Review Documentation
Show you've done thorough self-review:

```markdown
## Self-Review Notes

**Code Quality:**
- Ran `cargo clippy` and addressed all warnings
- Added comprehensive error handling for all OAuth2 flows
- Included debug logging for troubleshooting

**Testing Strategy:**
- Added both unit and integration tests
- Tested with real OAuth2 providers (not just mocks)
- Verified backwards compatibility with existing users

**Documentation:**
- Updated API docs with new authentication endpoints
- Added troubleshooting section for common OAuth2 issues
- Included examples in user guide
```

## Common Patterns

### Feature Additions
```markdown
## Description
This PR adds [feature] to enable [capability]. Users can now [specific benefit].

## Changes Made
- Added [main component]
- Modified [existing component] to support [new functionality]
- Updated [related systems] for integration

## Testing
- [x] New feature works in happy path scenarios
- [x] Error handling works for edge cases
- [x] Integration with existing features verified
- [x] Performance impact is acceptable
```

### Bug Fixes
```markdown
## Description
Fixes [specific issue] where [problem description]. The root cause was [brief explanation].

## Root Cause Analysis
[Detailed explanation of what caused the bug]

## Solution
[How the fix addresses the root cause]

## Testing
- [x] Verified fix resolves the reported issue
- [x] Added regression test to prevent future occurrences
- [x] Tested related functionality for side effects
```

### Refactoring
```markdown
## Description
Refactors [component] to improve [specific aspect] without changing functionality.

## Motivation
[Why this refactoring is needed]

## Changes Made
- Extracted [common logic] into [new structure]
- Improved [specific area] for better [benefit]
- No functional changes to user-facing behavior

## Testing
- [x] All existing tests pass unchanged
- [x] Verified no performance regression
- [x] Confirmed API behavior is identical
```

## Examples

### Excellent PR Description

```markdown
# feat(auth): implement comprehensive OAuth2 authentication system

## Description

This PR implements a complete OAuth2 authentication system supporting Google and GitHub providers. Users can now sign in using their existing accounts instead of creating new passwords, improving both security and user experience.

The implementation follows OAuth2 2.1 specifications and includes comprehensive error handling, token refresh logic, and account linking for existing users.

## Type of Change
- [x] New feature (non-breaking change which adds functionality)
- [x] Documentation update

## Related Issue
Fixes #145 - Implement social authentication
Relates to #132 - Improve user onboarding experience

## Changes Made

**Core Authentication:**
- Added OAuth2Service with provider abstraction
- Implemented GoogleOAuth2Provider and GitHubOAuth2Provider
- Created secure TokenManager with encryption and refresh logic
- Added AuthMiddleware for request validation

**Database Schema:**
- Extended User model with provider_id and provider_type fields
- Added oauth_tokens table for secure token storage
- Created migration scripts with rollback support

**User Interface:**
- Updated login page with OAuth2 provider buttons
- Added account linking flow for existing users
- Implemented proper error messaging and loading states
- Added user profile section for managing connected accounts

**API Endpoints:**
- POST /auth/oauth/initiate - Start OAuth2 flow
- GET /auth/oauth/callback - Handle provider callbacks
- POST /auth/oauth/link - Link provider to existing account
- DELETE /auth/oauth/unlink - Remove provider connection

## Testing

**Automated Testing:**
- [x] All existing tests pass (98% coverage maintained)
- [x] Added comprehensive OAuth2 test suite (45 new tests)
- [x] Integration tests for both Google and GitHub flows
- [x] Security tests for token handling and validation

**Manual Testing:**
- [x] Complete sign-up flow with Google OAuth2
- [x] Complete sign-up flow with GitHub OAuth2
- [x] Account linking for users with existing passwords
- [x] Account unlinking and re-linking scenarios
- [x] Error scenarios (user cancellation, network failures, invalid tokens)
- [x] Token refresh and expiration handling
- [x] Cross-browser compatibility (Chrome, Firefox, Safari)

**Security Testing:**
- [x] CSRF protection verified
- [x] Token encryption and secure storage validated
- [x] No sensitive data in logs or error messages
- [x] Rate limiting on authentication endpoints

### Test Commands
```bash
# Run full test suite
cargo test

# Run OAuth2-specific tests
cargo test oauth

# Run security-focused tests
cargo test auth::security

# Manual testing endpoints
curl -X POST http://localhost:3000/auth/oauth/initiate \
  -H "Content-Type: application/json" \
  -d '{"provider": "google"}'
```

## Screenshots
![OAuth2 Login Flow](screenshots/oauth2-login.png)
![Account Linking Interface](screenshots/account-linking.png)

## Checklist
- [x] My code follows the project's style guidelines
- [x] I have performed a self-review of my own code
- [x] I have commented my code, particularly in hard-to-understand areas
- [x] I have made corresponding changes to the documentation
- [x] My changes generate no new warnings
- [x] I have added tests that prove my fix is effective or that my feature works
- [x] New and existing unit tests pass locally with my changes
- [x] Any dependent changes have been merged and published

## Security Considerations
- OAuth2 tokens are encrypted before database storage
- PKCE (Proof Key for Code Exchange) implemented for additional security
- Token refresh happens automatically with secure rotation
- All authentication endpoints include rate limiting
- Comprehensive audit logging for security events

## Performance Impact
- Added database queries optimized with proper indexing
- Token validation cached for 5 minutes to reduce provider API calls
- No impact on existing authentication flows
- OAuth2 flows complete in under 2 seconds on average

## Breaking Changes
None. This is a purely additive feature that doesn't affect existing functionality.

## Additional Notes

**Future Enhancements:**
- Additional OAuth2 providers can be easily added using the provider interface
- Token storage supports encryption key rotation for enhanced security
- The system is designed to support enterprise SSO providers in the future

**Documentation:**
- Updated README with OAuth2 setup instructions
- Added troubleshooting guide for common OAuth2 issues
- Created user guide with screenshots for account linking

**Deployment Notes:**
- Requires new environment variables for OAuth2 client credentials
- Database migration will run automatically
- No downtime required for deployment
```

This example demonstrates:
- Clear, specific title following conventional commits
- Comprehensive description with context and impact
- Detailed changes organized by category
- Thorough testing documentation
- Security and performance considerations
- Forward-thinking additional notes

## Conclusion

Following these guidelines will help create pull requests that:
- Are easy and efficient to review
- Provide clear documentation of changes
- Ensure code quality and project standards
- Create valuable project history
- Facilitate team collaboration

Remember: A few extra minutes writing a good PR description can save hours of reviewer time and prevent future confusion.