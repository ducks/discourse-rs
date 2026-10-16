# Live updates over /bus

For authors of bots, integrations and apps that follow a forum live.
Discourse serves this through MessageBus at `/message-bus/:client_id/poll`.
discourse-rs does not implement that protocol (ROADMAP item 100, dropped);
it serves the same channels and payloads over its own bus, pg-bus, at
`/bus`. A client written against MessageBus needs a small transport change,
not new message handling.

## Endpoints

Both take `channels`, a comma-separated list, and an optional `position`.

- `GET /bus/events` is a server-sent events stream.
- `GET /bus/poll` is a long poll for networks or clients that cannot
  stream.

Channel names are Discourse's: `/topic/35`, `/latest`, `/new`,
`/unread/3`, `/notification/3`, `/notification-alert/3`,
`/private-message-topic-tracking-state/user/3`, `/chat/7`,
`/presence/chat-reply/7` and so on. Each message's `data` is the payload
Rails publishes on that channel. Channels match exactly, with no wildcards.

## Authentication

The same as the JSON API:

- the `_t` session cookie of a logged-in browser;
- an admin API key in `Api-Key` (with `Api-Username` or `Api-User-Id` for
  an all-users key). Granular scopes do not cover `/bus`, so use a key
  without scopes;
- a user API key in `User-Api-Key`, with the `read` or `write` scope. The
  `message_bus` and `notifications` scopes match only Rails'
  `/message-bus/.../poll` path, so they do not cover `/bus`.

No credentials subscribes as an anonymous viewer.

What a subscriber hears is decided on the server, not by the channel list.
A message limited to some users or groups (MessageBus's `user_ids` and
`group_ids`) reaches only subscribers who are among them. The groups
follow MessageBus.group_ids_lookup: an admin is in every group, a member
in their own plus logged_in_users (and everyone, unless
`granular_anonymous_and_logged_in_groups_permissions` is on), an
anonymous viewer in anonymous_users. Asking for a channel you cannot hear
is not an error; its messages are just not delivered. Access is checked
when a stream or poll starts, so a stream that ends and reconnects picks
up changed permissions.

## Positions

A position is an opaque string (currently `<xid>-<id>`) marking a point
in the whole bus, not in one channel. Store it as given and send it back.
The stream starts after:

1. the `Last-Event-ID` header, if present and well formed;
2. else the `position` parameter (a malformed one is a 400);
3. else now.

To start without missing anything, open the stream (or start the poll)
before loading the state it updates, then apply messages to that state.
A message may repeat something the load already showed; it is never
skipped. The port's own HTML pages carry the position they were rendered
at in `<meta name="bus-position">`.

There are no per-channel ids. Fields Rails fills from MessageBus ids
(`message_bus_last_id` in a topic, chat's `message_bus_last_ids`, user
status's `message_bus_last_id`, `do_not_disturb_channel_position`) are
placeholders here: do not resume from them. A presence state's
`last_message_id` is a bus position and can be used as `position`.

## The stream

`GET /bus/events?channels=/latest,/notification/3`

It starts with `retry: 1000`. Each message is an event of type `message`
whose `id` is its position and whose data is
`{"channel": "...", "data": ...}`. A comment arrives every 25 seconds to
keep connections open. The stream ends after about 10 minutes (spread by
10% either way); reconnect with `Last-Event-ID` set to the last id seen.
A browser `EventSource` does this by itself; other clients must send the
header or pass the id as `position`.

An event of type `gap` (data `{}`, no id) means messages after your
position were trimmed before you read them. Reload the state from the
JSON API and carry on from the stream.

## The poll

`GET /bus/poll?channels=/latest,/notification/3&position=...`

It answers at once when messages are due, else with the first that
arrives within 25 seconds (and any that came with it), else empty:

```json
{
  "messages": [
    {"id": "1234-56", "channel": "/latest", "data": {"message_type": "latest"}}
  ],
  "position": "1234-56",
  "gap": false
}
```

Poll again with `position` set to the answer's `position`. `gap: true`
means the same as the stream's `gap` event.

## Delivery

Messages are published inside the transaction that makes the change and
delivered once it commits, never if it rolls back. All subscribers see
them in the same order. A long-running write transaction in the database
can delay delivery until it ends; nothing is lost.
