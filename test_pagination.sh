#!/usr/bin/env bash

set -e

echo "=== Testing Pagination ==="
echo ""

# Register a user and get token
echo "1. Registering test user..."
RESPONSE=$(curl -s -X POST 'http://127.0.0.1:8080/api/auth/register' \
  -H 'Content-Type: application/json' \
  -d '{"username":"testuser1","email":"test1@example.com","password":"password123"}')
TOKEN=$(echo "$RESPONSE" | jq -r '.token')
echo "Got token: ${TOKEN:0:20}..."
echo ""

# Create a topic
echo "2. Creating test topic..."
TOPIC_RESPONSE=$(curl -s -X POST 'http://127.0.0.1:8080/api/topics' \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"title":"Test Topic","user_id":1,"category_id":1}')
TOPIC_ID=$(echo "$TOPIC_RESPONSE" | jq -r '.id')
echo "Created topic ID: $TOPIC_ID"
echo ""

# Create 15 posts in that topic
echo "3. Creating 15 test posts..."
for i in {1..15}; do
  curl -s -X POST 'http://127.0.0.1:8080/api/posts' \
    -H 'Content-Type: application/json' \
    -H "Authorization: Bearer $TOKEN" \
    -d "{\"topic_id\":$TOPIC_ID,\"user_id\":1,\"raw\":\"Test post number $i\",\"cooked\":\"<p>Test post number $i</p>\",\"post_number\":$i}" > /dev/null
  echo "  Created post $i"
done
echo ""

# Test pagination
echo "4. Testing pagination on /api/posts..."
echo ""

echo "   a) Default (should return up to 30):"
COUNT=$(curl -s 'http://127.0.0.1:8080/api/posts' | jq 'length')
echo "      Returned $COUNT posts"
echo ""

echo "   b) Page 1, 5 per page:"
POSTS=$(curl -s 'http://127.0.0.1:8080/api/posts?page=1&per_page=5' | jq -r '.[].id')
COUNT=$(echo "$POSTS" | wc -l)
echo "      Returned $COUNT posts with IDs: $(echo $POSTS | tr '\n' ' ')"
echo ""

echo "   c) Page 2, 5 per page:"
POSTS=$(curl -s 'http://127.0.0.1:8080/api/posts?page=2&per_page=5' | jq -r '.[].id')
COUNT=$(echo "$POSTS" | wc -l)
echo "      Returned $COUNT posts with IDs: $(echo $POSTS | tr '\n' ' ')"
echo ""

echo "   d) Page 3, 5 per page:"
POSTS=$(curl -s 'http://127.0.0.1:8080/api/posts?page=3&per_page=5' | jq -r '.[].id')
COUNT=$(echo "$POSTS" | wc -l)
echo "      Returned $COUNT posts with IDs: $(echo $POSTS | tr '\n' ' ')"
echo ""

echo "   e) Per page = 100 (should be clamped to max 100):"
COUNT=$(curl -s 'http://127.0.0.1:8080/api/posts?per_page=100' | jq 'length')
echo "      Returned $COUNT posts"
echo ""

echo "   f) Per page = 200 (should be clamped to max 100):"
COUNT=$(curl -s 'http://127.0.0.1:8080/api/posts?per_page=200' | jq 'length')
echo "      Returned $COUNT posts (clamped to 100)"
echo ""

echo "=== Pagination test complete! ==="
