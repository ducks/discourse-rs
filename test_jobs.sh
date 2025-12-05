#!/usr/bin/env bash

# Background jobs test script

set -e

BASE_URL="http://127.0.0.1:8080"

echo "========================================="
echo "Testing Background Jobs"
echo "========================================="
echo ""

# First, register a user and get a token
echo "1. Registering test user..."
RESPONSE=$(curl -s -X POST "$BASE_URL/api/auth/register" \
  -H "Content-Type: application/json" \
  -d '{"username":"jobtest","email":"jobtest@example.com","password":"password123"}')

TOKEN=$(echo "$RESPONSE" | jq -r '.token')
echo "Token: ${TOKEN:0:20}..."
echo ""

# Create a topic
echo "2. Creating a test topic..."
TOPIC_RESPONSE=$(curl -s -X POST "$BASE_URL/api/topics" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"title":"Test Topic for Jobs","user_id":1}')

TOPIC_ID=$(echo "$TOPIC_RESPONSE" | jq -r '.id')
echo "Created topic ID: $TOPIC_ID"
echo ""

# Enqueue welcome email job
echo "3. Enqueueing welcome email job..."
WELCOME_JOB=$(curl -s -X POST "$BASE_URL/api/jobs/welcome_email" \
  -H "Content-Type: application/json" \
  -d '{
    "user_id": 1,
    "username": "jobtest",
    "email": "jobtest@example.com"
  }')

echo "Welcome email job response:"
echo "$WELCOME_JOB" | jq '.'
echo ""

# Enqueue process topic job
echo "4. Enqueueing process topic job..."
PROCESS_JOB=$(curl -s -X POST "$BASE_URL/api/jobs/process_topic" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC_ID,
    \"action\": \"index\"
  }")

echo "Process topic job response:"
echo "$PROCESS_JOB" | jq '.'
echo ""

echo "5. Jobs enqueued! Check server logs to see them being processed."
echo "   Look for messages like:"
echo "   - 'Starting background job worker pool...'"
echo "   - 'Sending welcome email to user...'"
echo "   - 'Processing topic...'"
echo ""

echo "========================================="
echo "Background Jobs Test Complete!"
echo "========================================="
