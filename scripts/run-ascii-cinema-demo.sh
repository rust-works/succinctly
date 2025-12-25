#!/bin/bash

# ASCII Cinema Demo Script for omni-dev
# Run this from the demo repository after setting it up

set -e

# Colors for better visual appeal
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
PURPLE='\033[0;35m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Function to simulate typing
type_text() {
    local text="$1"
    local delay="${2:-0.05}"
    for (( i=0; i<${#text}; i++ )); do
        printf "${text:$i:1}"
        sleep "$delay"
    done
    echo
}

# Function to pause with visual indicator
pause() {
    local duration="${1:-2}"
    echo -e "${YELLOW}⏳ Pausing for ${duration}s...${NC}"
    sleep "$duration"
}

# Check if we're in the demo repository
if [[ ! -f "src/auth/oauth.js" ]]; then
    echo -e "${RED}❌ Error: Please run this script from the demo repository directory${NC}"
    echo -e "${YELLOW}💡 Run: cd cinema/omni-dev-demo && ../../scripts/run-ascii-cinema-demo.sh${NC}"
    exit 1
fi

clear
echo -e "${PURPLE}========================================${NC}"
echo -e "${PURPLE}  🎬 omni-dev ASCII Cinema Demo 🎬     ${NC}"
echo -e "${PURPLE}========================================${NC}"
echo ""

type_text "Welcome to omni-dev - the AI-powered Git commit toolkit!" 0.03
echo -e "${CYAN}Let's see how it transforms messy commits into professional ones!${NC}"
echo ""
pause 2

# 1. Show current messy commit history
echo -e "${YELLOW}📊 First, let's look at our current commit history...${NC}"
type_text "git log --oneline -7" 0.05
git log --oneline -7
echo ""
echo -e "${RED}😱 Yikes! These commit messages are terrible!${NC}"
echo -e "${RED}   'wip', 'fix stuff', 'asdf' - not very helpful...${NC}"
pause 3

# 2. Show what we're working with
echo -e "${YELLOW}🔍 Let's see what changes we actually made...${NC}"
type_text "git diff HEAD~6..HEAD --stat" 0.05
git diff HEAD~6..HEAD --stat
echo ""
echo -e "${BLUE}We can see we modified authentication, API, UI, and docs${NC}"
pause 2

# 3. Use omni-dev to analyze and improve commits
echo -e "${GREEN}🤖 Now let's use omni-dev to analyze and improve these commits!${NC}"
echo ""
type_text "export CLAUDE_API_KEY='sk-ant-...'  # Set your Claude API key" 0.05
echo -e "${YELLOW}💡 (Make sure you have your Claude API key configured)${NC}"
echo ""
pause 1

type_text "omni-dev git commit message twiddle 'HEAD~6..HEAD' --use-context" 0.05
echo ""
echo -e "${CYAN}🧠 AI is analyzing the code changes and improving commit messages...${NC}"
echo -e "${CYAN}   This may take a few moments as AI processes each commit...${NC}"
pause 3

# Simulate the AI processing (in real demo, this would actually run)
echo -e "${GREEN}✨ AI Analysis Complete! Here's what happened:${NC}"
echo ""
echo -e "${BLUE}Commit 1: 'wip' → 'feat(auth): implement OAuth2 token validation'${NC}"
echo -e "${BLUE}Commit 2: 'fix stuff' → 'feat(api): add TODO comments for error handling'${NC}"
echo -e "${BLUE}Commit 3: 'update files' → 'style(ui): add CSS comments for LoginForm component'${NC}"
echo -e "${BLUE}Commit 4: 'asdf' → 'docs(api): expand API documentation with examples'${NC}"
echo -e "${BLUE}Commit 5: 'changes' → 'feat(auth): add token validation method to OAuth2Client'${NC}"
echo -e "${BLUE}Commit 6: 'mobile fix' → 'feat(ui): implement responsive design for mobile devices'${NC}"
echo -e "${BLUE}Commit 7: 'docs update' → 'docs(api): add comprehensive error handling documentation'${NC}"
pause 4

# 4. Show the improved commit history
echo -e "${GREEN}🎉 Let's see our beautiful new commit history!${NC}"
type_text "git log --oneline -7" 0.05
echo ""
echo -e "${GREEN}feat(auth): implement OAuth2 token validation${NC}"
echo -e "${GREEN}feat(api): add TODO comments for error handling${NC}"
echo -e "${GREEN}style(ui): add CSS comments for LoginForm component${NC}"
echo -e "${GREEN}docs(api): expand API documentation with examples${NC}"
echo -e "${GREEN}feat(auth): add token validation method to OAuth2Client${NC}"
echo -e "${GREEN}feat(ui): implement responsive design for mobile devices${NC}"
echo -e "${GREEN}docs(api): add comprehensive error handling documentation${NC}"
echo ""
echo -e "${GREEN}✨ Amazing! Professional, descriptive commit messages!${NC}"
pause 3

# 5. Show commit analysis feature
echo -e "${YELLOW}🔍 Let's analyze one of our commits in detail...${NC}"
type_text "omni-dev git commit analyze HEAD" 0.05
echo ""
echo -e "${CYAN}📊 Detailed Commit Analysis:${NC}"
echo -e "${BLUE}  • Type: Documentation${NC}"
echo -e "${BLUE}  • Scope: API${NC}"
echo -e "${BLUE}  • Impact: Medium${NC}"
echo -e "${BLUE}  • Files Modified: 1${NC}"
echo -e "${BLUE}  • Lines Added: 12${NC}"
echo -e "${BLUE}  • Conventional Commit: ✅ Yes${NC}"
pause 3

# 6. Show branch analysis
echo -e "${YELLOW}🌿 Now let's analyze our entire branch...${NC}"
type_text "omni-dev git branch analyze" 0.05
echo ""
echo -e "${CYAN}📈 Branch Analysis Summary:${NC}"
echo -e "${BLUE}  • Total Commits: 7${NC}"
echo -e "${BLUE}  • Features: 3${NC}"
echo -e "${BLUE}  • Documentation: 2${NC}"
echo -e "${BLUE}  • Styles: 1${NC}"
echo -e "${BLUE}  • Bug Fixes: 0${NC}"
echo -e "${BLUE}  • Conventional Commits: 100%${NC}"
echo -e "${GREEN}  • Branch Quality: Excellent ⭐⭐⭐⭐⭐${NC}"
pause 3

# 7. Create a professional PR
echo -e "${PURPLE}🚀 Finally, let's create a professional PR with AI-generated description!${NC}"
type_text "omni-dev git branch create pr" 0.05
echo ""
echo -e "${CYAN}🤖 AI is generating professional PR description...${NC}"
pause 2
echo ""
echo -e "${GREEN}✅ Pull Request Created Successfully!${NC}"
echo ""
echo -e "${BLUE}Title: feat: implement OAuth2 authentication and responsive UI${NC}"
echo ""
echo -e "${BLUE}Description:${NC}"
echo -e "${BLUE}## 🚀 Features${NC}"
echo -e "${BLUE}- Implement OAuth2 token validation system${NC}"
echo -e "${BLUE}- Add responsive design for mobile devices${NC}"
echo -e "${BLUE}- Enhance API error handling architecture${NC}"
echo ""
echo -e "${BLUE}## 📚 Documentation${NC}"
echo -e "${BLUE}- Expand API documentation with examples${NC}"
echo -e "${BLUE}- Add comprehensive error handling guide${NC}"
echo ""
echo -e "${BLUE}## 🧪 Testing${NC}"
echo -e "${BLUE}- [ ] Test OAuth2 token validation${NC}"
echo -e "${BLUE}- [ ] Verify responsive design on mobile${NC}"
echo -e "${BLUE}- [ ] Test API error responses${NC}"
pause 4

# 8. Final showcase
echo ""
echo -e "${PURPLE}========================================${NC}"
echo -e "${GREEN}          🎉 Demo Complete! 🎉         ${NC}"
echo -e "${PURPLE}========================================${NC}"
echo ""
echo -e "${CYAN}What we accomplished:${NC}"
echo -e "${GREEN}✅ Transformed 7 messy commits into professional ones${NC}"
echo -e "${GREEN}✅ Generated conventional commit messages${NC}"
echo -e "${GREEN}✅ Analyzed commit and branch quality${NC}"
echo -e "${GREEN}✅ Created a professional PR with AI description${NC}"
echo ""
echo -e "${YELLOW}🚀 omni-dev: Making your Git history professional, one commit at a time!${NC}"
echo ""
echo -e "${BLUE}Installation:${NC}"
echo -e "${BLUE}  cargo install omni-dev${NC}"
echo -e "${BLUE}  # or${NC}"
echo -e "${BLUE}  nix profile install github:rust-works/omni-dev${NC}"
echo ""
echo -e "${BLUE}Learn more: https://github.com/rust-works/omni-dev${NC}"