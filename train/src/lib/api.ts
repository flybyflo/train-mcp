import Constants from 'expo-constants';

const FALLBACK_PORT = '8081';

export function generateAPIUrl(path: string) {
  const hostUri = Constants.expoConfig?.hostUri;
  if (!hostUri) {
    return `http://localhost:${FALLBACK_PORT}${path}`;
  }

  const host = hostUri.split(':')[0];
  return `http://${host}:${FALLBACK_PORT}${path}`;
}
