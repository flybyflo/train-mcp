import { createOpenAI } from '@ai-sdk/openai';
import { convertToModelMessages, streamText, type UIMessage } from 'ai';

type ChatRequestBody = {
  messages?: UIMessage[];
};

function jsonResponse(body: unknown, status: number) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

export async function POST(request: Request) {
  const apiKey = process.env.OPENROUTER_API_KEY;
  if (!apiKey) {
    return jsonResponse(
      { error: 'Missing OPENROUTER_API_KEY. Add it to your local environment.' },
      500,
    );
  }

  let body: ChatRequestBody;
  try {
    body = (await request.json()) as ChatRequestBody;
  } catch {
    return jsonResponse({ error: 'Invalid JSON request body.' }, 400);
  }

  const messages = body.messages ?? [];
  const modelId = process.env.OPENROUTER_MODEL ?? 'openai/gpt-4.1-mini';

  const openrouter = createOpenAI({
    name: 'openrouter',
    apiKey,
    baseURL: 'https://openrouter.ai/api/v1',
    headers: {
      ...(process.env.OPENROUTER_SITE_URL
        ? { 'HTTP-Referer': process.env.OPENROUTER_SITE_URL }
        : {}),
      ...(process.env.OPENROUTER_APP_NAME ? { 'X-Title': process.env.OPENROUTER_APP_NAME } : {}),
    },
  });

  const modelMessages = await convertToModelMessages(messages);

  const result = streamText({
    model: openrouter(modelId),
    messages: modelMessages,
  });

  return result.toUIMessageStreamResponse();
}
