#!/usr/bin/env bash
set -e

API_BASE="http://127.0.0.1:8080/api"

echo "=== Seeding Test Data for discourse-rs ==="
echo ""

# Create test user
echo "Creating test user..."
REGISTER_RESPONSE=$(curl -s -X POST "$API_BASE/auth/register" \
  -H "Content-Type: application/json" \
  -d '{
    "username": "test",
    "email": "test@example.com",
    "password": "test123"
  }')

TOKEN=$(echo "$REGISTER_RESPONSE" | jq -r '.token // empty')

if [ -z "$TOKEN" ]; then
  echo "User might already exist, trying to login..."
  LOGIN_RESPONSE=$(curl -s -X POST "$API_BASE/auth/login" \
    -H "Content-Type: application/json" \
    -d '{
      "username": "test",
      "password": "test123"
    }')

  TOKEN=$(echo "$LOGIN_RESPONSE" | jq -r '.token // empty')

  if [ -z "$TOKEN" ]; then
    echo "Failed to get authentication token!"
    exit 1
  fi
fi

echo "✓ Test user ready (username: test, password: test123)"
echo ""

# Create additional test users
echo "Creating additional test users..."

# User 2: alice
ALICE_RESPONSE=$(curl -s -X POST "$API_BASE/auth/register" \
  -H "Content-Type: application/json" \
  -d '{
    "username": "alice",
    "email": "alice@example.com",
    "password": "alice123"
  }')
ALICE_TOKEN=$(echo "$ALICE_RESPONSE" | jq -r '.token // empty')
[ -z "$ALICE_TOKEN" ] && ALICE_TOKEN=$(curl -s -X POST "$API_BASE/auth/login" \
  -H "Content-Type: application/json" \
  -d '{"username":"alice","password":"alice123"}' | jq -r '.token')
echo "✓ Created user: alice"

# User 3: bob
BOB_RESPONSE=$(curl -s -X POST "$API_BASE/auth/register" \
  -H "Content-Type: application/json" \
  -d '{
    "username": "bob",
    "email": "bob@example.com",
    "password": "bob123"
  }')
BOB_TOKEN=$(echo "$BOB_RESPONSE" | jq -r '.token // empty')
[ -z "$BOB_TOKEN" ] && BOB_TOKEN=$(curl -s -X POST "$API_BASE/auth/login" \
  -H "Content-Type: application/json" \
  -d '{"username":"bob","password":"bob123"}' | jq -r '.token')
echo "✓ Created user: bob"

# User 4: charlie
CHARLIE_RESPONSE=$(curl -s -X POST "$API_BASE/auth/register" \
  -H "Content-Type: application/json" \
  -d '{
    "username": "charlie",
    "email": "charlie@example.com",
    "password": "charlie123"
  }')
CHARLIE_TOKEN=$(echo "$CHARLIE_RESPONSE" | jq -r '.token // empty')
[ -z "$CHARLIE_TOKEN" ] && CHARLIE_TOKEN=$(curl -s -X POST "$API_BASE/auth/login" \
  -H "Content-Type: application/json" \
  -d '{"username":"charlie","password":"charlie123"}' | jq -r '.token')
echo "✓ Created user: charlie"

echo ""
echo "Note: Set trust_level=4 in database to make test user a moderator"
echo "Example: psql \$DATABASE_URL -c \"UPDATE users SET trust_level = 4 WHERE username = 'test';\""
echo ""

# Create categories
echo "Creating categories..."

# General category
GENERAL_CAT=$(curl -s -X POST "$API_BASE/categories" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "General",
    "slug": "general",
    "description": "General discussion topics",
    "color": "0088CC",
    "position": 1
  }')
GENERAL_CAT_ID=$(echo "$GENERAL_CAT" | jq -r '.id // empty')
echo "✓ Created category: General (ID: $GENERAL_CAT_ID)"

# Technology category
TECH_CAT=$(curl -s -X POST "$API_BASE/categories" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Technology",
    "slug": "technology",
    "description": "Tech news, programming, and development",
    "color": "25AAE2",
    "position": 2
  }')
TECH_CAT_ID=$(echo "$TECH_CAT" | jq -r '.id // empty')
echo "✓ Created category: Technology (ID: $TECH_CAT_ID)"

# Music category
MUSIC_CAT=$(curl -s -X POST "$API_BASE/categories" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Music",
    "slug": "music",
    "description": "Music discussion, recommendations, and more",
    "color": "ED207B",
    "position": 3
  }')
MUSIC_CAT_ID=$(echo "$MUSIC_CAT" | jq -r '.id // empty')
echo "✓ Created category: Music (ID: $MUSIC_CAT_ID)"

# Gaming category
GAMING_CAT=$(curl -s -X POST "$API_BASE/categories" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Gaming",
    "slug": "gaming",
    "description": "Video games, board games, and gaming culture",
    "color": "92278F",
    "position": 4
  }')
GAMING_CAT_ID=$(echo "$GAMING_CAT" | jq -r '.id // empty')
echo "✓ Created category: Gaming (ID: $GAMING_CAT_ID)"

# Meta category
META_CAT=$(curl -s -X POST "$API_BASE/categories" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Meta",
    "slug": "meta",
    "description": "Discussion about this forum itself",
    "color": "808281",
    "position": 5
  }')
META_CAT_ID=$(echo "$META_CAT" | jq -r '.id // empty')
echo "✓ Created category: Meta (ID: $META_CAT_ID)"

echo ""

# Create topics and posts
echo "Creating topics and posts..."
echo ""

# Topic 1: Welcome to the forum (General)
echo "Creating topic: Welcome to the forum..."
TOPIC1=$(curl -s -X POST "$API_BASE/topics" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"title\": \"Welcome to the forum!\",
    \"category_id\": $GENERAL_CAT_ID,
    \"content\": \"Welcome everyone! This is a place for friendly discussion and community building. Feel free to introduce yourself and explore the different categories.\"
  }")
TOPIC1_ID=$(echo "$TOPIC1" | jq -r '.id // empty')
echo "✓ Created topic ID: $TOPIC1_ID"

# Reply to Topic 1 from Alice
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $ALICE_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC1_ID,
    \"content\": \"Thanks for the warm welcome! I'm Alice, excited to be here and meet everyone.\"
  }" > /dev/null && echo "  ✓ Reply from alice"

# Reply to Topic 1 from Bob
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $BOB_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC1_ID,
    \"content\": \"Hey everyone! Bob here. Looking forward to some great discussions!\"
  }" > /dev/null && echo "  ✓ Reply from bob"

echo ""

# Topic 2: Learning Rust (Technology)
echo "Creating topic: Learning Rust..."
TOPIC2=$(curl -s -X POST "$API_BASE/topics" \
  -H "Authorization: Bearer $ALICE_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"title\": \"Learning Rust: Tips and Resources\",
    \"category_id\": $TECH_CAT_ID,
    \"content\": \"I've been learning Rust for a few months now. What are your favorite resources? I've been using the Rust Book and Rustlings. Any other recommendations?\"
  }")
TOPIC2_ID=$(echo "$TOPIC2" | jq -r '.id // empty')
echo "✓ Created topic ID: $TOPIC2_ID"

# Reply to Topic 2 from Test
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC2_ID,
    \"content\": \"Great question! I highly recommend 'Rust by Example' and the official Rust docs. Also, building small projects really helps solidify the concepts.\"
  }" > /dev/null && echo "  ✓ Reply from test"

# Reply to Topic 2 from Charlie
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $CHARLIE_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC2_ID,
    \"content\": \"Don't forget about the Rust subreddit and Discord! The community is super helpful when you get stuck.\"
  }" > /dev/null && echo "  ✓ Reply from charlie"

# Reply to Topic 2 from Bob
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $BOB_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC2_ID,
    \"content\": \"I've been doing Advent of Code challenges in Rust. It's a fun way to practice!\"
  }" > /dev/null && echo "  ✓ Reply from bob"

# Threaded reply to Alice's original post (post_number 1)
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC2_ID,
    \"content\": \"@alice Have you checked out 'Programming Rust' by O'Reilly? It's great for intermediate topics like concurrency.\",
    \"reply_to_post_number\": 1
  }" > /dev/null && echo "  ✓ Threaded reply from test to alice's post"

echo ""

# Topic 3: Album Recommendations (Music)
echo "Creating topic: Album Recommendations..."
TOPIC3=$(curl -s -X POST "$API_BASE/topics" \
  -H "Authorization: Bearer $BOB_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"title\": \"What albums are you listening to lately?\",
    \"category_id\": $MUSIC_CAT_ID,
    \"content\": \"I've been really into the new Thundercat album. What's everyone else been listening to? Drop your recommendations!\"
  }")
TOPIC3_ID=$(echo "$TOPIC3" | jq -r '.id // empty')
echo "✓ Created topic ID: $TOPIC3_ID"

# Reply to Topic 3 from Charlie
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $CHARLIE_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC3_ID,
    \"content\": \"I've had the new Kendrick album on repeat. Also been revisiting some classic Todd Snider stuff.\"
  }" > /dev/null && echo "  ✓ Reply from charlie"

# Reply to Topic 3 from Alice
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $ALICE_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC3_ID,
    \"content\": \"Sierra Ferrell's latest is amazing! If you're into folk/country with a twist, definitely check it out.\"
  }" > /dev/null && echo "  ✓ Reply from alice"

echo ""

# Topic 4: Favorite Indie Games (Gaming)
echo "Creating topic: Favorite Indie Games..."
TOPIC4=$(curl -s -X POST "$API_BASE/topics" \
  -H "Authorization: Bearer $CHARLIE_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"title\": \"What are your favorite indie games?\",
    \"category_id\": $GAMING_CAT_ID,
    \"content\": \"I just finished Hades and it was incredible. What indie games have you been playing? Looking for recommendations!\"
  }")
TOPIC4_ID=$(echo "$TOPIC4" | jq -r '.id // empty')
echo "✓ Created topic ID: $TOPIC4_ID"

# Reply to Topic 4 from Test
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC4_ID,
    \"content\": \"Hades is amazing! Have you tried Hollow Knight? It's challenging but so rewarding.\"
  }" > /dev/null && echo "  ✓ Reply from test"

# Reply to Topic 4 from Alice
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $ALICE_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC4_ID,
    \"content\": \"I've been playing Stardew Valley for the millionth time. It's just so relaxing!\"
  }" > /dev/null && echo "  ✓ Reply from alice"

# Reply to Topic 4 from Bob
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $BOB_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC4_ID,
    \"content\": \"Celeste is a masterpiece. Great story, fantastic gameplay, and an amazing soundtrack.\"
  }" > /dev/null && echo "  ✓ Reply from bob"

# Threaded reply to Test's Hollow Knight recommendation
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $CHARLIE_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC4_ID,
    \"content\": \"@test Hollow Knight is amazing! The art style is gorgeous too. Did you find all the secret areas?\",
    \"reply_to_post_number\": 2
  }" > /dev/null && echo "  ✓ Threaded reply from charlie to test's post"

echo ""

# Topic 5: Feature Requests (Meta)
echo "Creating topic: Feature Requests..."
TOPIC5=$(curl -s -X POST "$API_BASE/topics" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"title\": \"Feature Requests and Suggestions\",
    \"category_id\": $META_CAT_ID,
    \"content\": \"What features would you like to see on this forum? Let's collect ideas and discuss what would make this a better community space.\"
  }")
TOPIC5_ID=$(echo "$TOPIC5" | jq -r '.id // empty')
echo "✓ Created topic ID: $TOPIC5_ID"

# Reply to Topic 5 from Bob
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $BOB_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC5_ID,
    \"content\": \"It would be great to have notifications when someone replies to your posts!\"
  }" > /dev/null && echo "  ✓ Reply from bob"

# Reply to Topic 5 from Charlie
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $CHARLIE_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC5_ID,
    \"content\": \"User profiles with bio and avatar support would be nice.\"
  }" > /dev/null && echo "  ✓ Reply from charlie"

echo ""

# Topic 6: API Development Discussion (Technology)
echo "Creating topic: API Development Discussion..."
TOPIC6=$(curl -s -X POST "$API_BASE/topics" \
  -H "Authorization: Bearer $BOB_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"title\": \"REST vs GraphQL: What do you prefer?\",
    \"category_id\": $TECH_CAT_ID,
    \"content\": \"I'm designing a new API and trying to decide between REST and GraphQL. What are your experiences with each? Pros and cons?\"
  }")
TOPIC6_ID=$(echo "$TOPIC6" | jq -r '.id // empty')
echo "✓ Created topic ID: $TOPIC6_ID"

# Reply to Topic 6 from Test
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC6_ID,
    \"content\": \"I prefer REST for simplicity. GraphQL is powerful but can be overkill for smaller projects. Really depends on your use case!\"
  }" > /dev/null && echo "  ✓ Reply from test"

# Reply to Topic 6 from Alice
curl -s -X POST "$API_BASE/posts" \
  -H "Authorization: Bearer $ALICE_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"topic_id\": $TOPIC6_ID,
    \"content\": \"GraphQL is great when you need flexible queries and have a complex data model. The tooling has gotten really good too.\"
  }" > /dev/null && echo "  ✓ Reply from alice"

echo ""
echo "=== Seed Data Complete ==="
echo ""
echo "Created:"
echo "  • 4 users (test, alice, bob, charlie)"
echo "  • 5 categories (General, Technology, Music, Gaming, Meta)"
echo "  • 6 topics with multiple replies"
echo "  • ~20 posts total"
echo ""
echo "Test user credentials:"
echo "  test / test123 (main moderator account)"
echo "  alice / alice123"
echo "  bob / bob123"
echo "  charlie / charlie123"
echo ""
echo "Auth token for test user: $TOKEN"
