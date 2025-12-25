#!/bin/bash

# ASCII Cinema Demo Setup Script for omni-dev
# This creates a demo repository with messy commits to showcase omni-dev's capabilities

set -e

# Create cinema directory if it doesn't exist
CINEMA_DIR="cinema"
DEMO_DIR="$CINEMA_DIR/omni-dev-demo"

echo "🎬 Setting up omni-dev demo environment..."

# Create cinema directory if it doesn't exist
mkdir -p "$CINEMA_DIR"

# Clean up any existing demo
rm -rf "$DEMO_DIR"

# Create demo project
mkdir -p "$DEMO_DIR"
cd "$DEMO_DIR"

# Initialize git repo
git init
git config user.name "Demo User"
git config user.email "demo@example.com"

# Create a realistic project structure
mkdir -p src/auth src/api src/ui docs tests
cat > README.md << 'EOF'
# Demo Project

A sample web application for demonstrating omni-dev capabilities.

## Features
- User authentication
- REST API
- React frontend
- Comprehensive testing
EOF

cat > src/auth/oauth.js << 'EOF'
// OAuth2 authentication implementation
class OAuth2Client {
  constructor(clientId, clientSecret) {
    this.clientId = clientId;
    this.clientSecret = clientSecret;
  }

  async authenticate(code) {
    // Implementation here
    return await this.exchangeCodeForToken(code);
  }

  async exchangeCodeForToken(code) {
    // Token exchange logic
    return { access_token: 'token', refresh_token: 'refresh' };
  }
}

module.exports = OAuth2Client;
EOF

cat > src/api/endpoints.js << 'EOF'
// REST API endpoints
const express = require('express');
const router = express.Router();

// User endpoints
router.get('/users', (req, res) => {
  res.json({ users: [] });
});

router.post('/users', (req, res) => {
  res.json({ message: 'User created' });
});

// Auth endpoints
router.post('/auth/login', (req, res) => {
  res.json({ token: 'jwt-token' });
});

module.exports = router;
EOF

cat > src/ui/LoginForm.jsx << 'EOF'
import React, { useState } from 'react';

const LoginForm = () => {
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');

  const handleSubmit = (e) => {
    e.preventDefault();
    // Login logic
  };

  return (
    <form onSubmit={handleSubmit} className="login-form">
      <input 
        type="email" 
        value={email}
        onChange={(e) => setEmail(e.target.value)}
        placeholder="Email"
      />
      <input 
        type="password"
        value={password} 
        onChange={(e) => setPassword(e.target.value)}
        placeholder="Password"
      />
      <button type="submit">Login</button>
    </form>
  );
};

export default LoginForm;
EOF

cat > docs/api.md << 'EOF'
# API Documentation

## Authentication Endpoints

### POST /auth/login
Login with email and password.

**Request:**
```json
{
  "email": "user@example.com",
  "password": "password123"
}
```

**Response:**
```json
{
  "token": "jwt-token-here",
  "user": {
    "id": 1,
    "email": "user@example.com"
  }
}
```

## User Endpoints

### GET /users
Get all users (admin only).

### POST /users  
Create a new user.
EOF

# Create initial commit
git add .
git commit -m "Initial project setup"

# Create main branch and push (simulate remote)
git branch -M main

# Now create messy commits to demonstrate omni-dev
echo "console.log('debug');" >> src/auth/oauth.js
git add .
git commit -m "wip"

echo "// TODO: fix this" >> src/api/endpoints.js
git add .
git commit -m "fix stuff"

echo "/* updated styles */" >> src/ui/LoginForm.jsx
git add .
git commit -m "update files"

echo "# More docs" >> docs/api.md
git add .
git commit -m "asdf"

cat >> src/auth/oauth.js << 'EOF'

// Add token validation
validateToken(token) {
  return token && token.length > 0;
}
EOF
git add .
git commit -m "changes"

cat >> src/ui/LoginForm.jsx << 'EOF'

// Add responsive styles
const styles = {
  '@media (max-width: 768px)': {
    width: '100%'
  }
};
EOF
git add .
git commit -m "mobile fix"

cat >> docs/api.md << 'EOF'

## Error Handling

All endpoints return consistent error responses:

```json
{
  "error": "Error message",
  "code": "ERROR_CODE"
}
```
EOF
git add .
git commit -m "docs update"

echo "🎯 Demo repository created with messy commits!"
echo "📁 Location: $(pwd)"
echo "📊 Commit history:"
git log --oneline

echo ""
echo "🎬 Ready to record ASCII cinema demo!"
echo "💡 Start recording with: asciinema rec ../../cinema/omni-dev-demo.cast"
echo "🚀 Then run: ../../scripts/run-ascii-cinema-demo.sh"