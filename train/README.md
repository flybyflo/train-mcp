# Train Expo App

Expo Router app with AI SDK chat using OpenRouter on `/api/chat`.

## Setup

1. Install dependencies:

```bash
npm install
```

2. Create local env file:

```bash
cp .env.example .env
```

3. Set `OPENROUTER_API_KEY` in `.env`.

4. Start the app:

```bash
npx expo start
```

5. Run iOS dev build:

```bash
npx expo run:ios
```

## Key files

- `src/app/index.tsx`: Chat UI using `useChat`.
- `src/app/api/chat+api.ts`: Server route that calls OpenRouter.
- `src/lib/api.ts`: API URL resolver for Expo iOS/dev.
