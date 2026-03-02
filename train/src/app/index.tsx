import { useChat } from '@ai-sdk/react';
import { DefaultChatTransport, type UIMessage } from 'ai';
import { fetch as expoFetch } from 'expo/fetch';
import { useState } from 'react';
import {
  ActivityIndicator,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  TextInput,
} from 'react-native';
import { SafeAreaView, useSafeAreaInsets } from 'react-native-safe-area-context';

import { ThemedText } from '@/components/themed-text';
import { ThemedView } from '@/components/themed-view';
import { BottomTabInset, MaxContentWidth, Spacing } from '@/constants/theme';
import { useTheme } from '@/hooks/use-theme';
import { generateAPIUrl } from '@/lib/api';

function getMessageText(message: UIMessage) {
  return message.parts
    .filter((part) => part.type === 'text')
    .map((part) => part.text)
    .join('');
}

export default function HomeScreen() {
  const [input, setInput] = useState('');
  const theme = useTheme();
  const safeAreaInsets = useSafeAreaInsets();
  const chatBottomInset = safeAreaInsets.bottom + BottomTabInset + Spacing.two;
  const { messages, sendMessage, status, error, stop } = useChat({
    transport: new DefaultChatTransport({
      api: generateAPIUrl('/api/chat'),
      fetch: expoFetch as typeof fetch,
    }),
  });

  const isSending = status === 'submitted' || status === 'streaming';

  function handleSend() {
    const text = input.trim();
    if (!text || isSending) {
      return;
    }

    sendMessage({ text });
    setInput('');
  }

  return (
    <ThemedView style={styles.container}>
      <SafeAreaView style={styles.safeArea}>
        <ThemedView style={styles.header}>
          <ThemedText type="subtitle">Train AI</ThemedText>
          <ThemedText themeColor="textSecondary">
            OpenRouter via AI SDK and Expo API routes
          </ThemedText>
        </ThemedView>

        <ScrollView
          style={styles.scrollView}
          contentContainerStyle={[styles.messagesContent, { paddingBottom: chatBottomInset }]}
          keyboardShouldPersistTaps="handled">
          {messages.length === 0 ? (
            <ThemedView type="backgroundElement" style={styles.emptyState}>
              <ThemedText themeColor="textSecondary">
                Ask a question to start a conversation.
              </ThemedText>
            </ThemedView>
          ) : (
            messages.map((message) => {
              const text = getMessageText(message);
              if (!text) return null;

              const isUser = message.role === 'user';
              return (
                <ThemedView
                  key={message.id}
                  type={isUser ? 'backgroundSelected' : 'backgroundElement'}
                  style={[styles.bubble, isUser ? styles.userBubble : styles.assistantBubble]}>
                  <ThemedText type="smallBold" style={styles.roleLabel}>
                    {isUser ? 'You' : 'Assistant'}
                  </ThemedText>
                  <ThemedText style={styles.messageText}>{text}</ThemedText>
                </ThemedView>
              );
            })
          )}

          {error ? (
            <ThemedView type="backgroundElement" style={styles.errorBox}>
              <ThemedText type="smallBold">Request failed</ThemedText>
              <ThemedText type="small" themeColor="textSecondary">
                {error.message}
              </ThemedText>
            </ThemedView>
          ) : null}
        </ScrollView>

        <KeyboardAvoidingView
          behavior={Platform.OS === 'ios' ? 'padding' : undefined}
          keyboardVerticalOffset={Spacing.two}>
          <ThemedView type="backgroundElement" style={styles.inputShell}>
            <TextInput
              style={[styles.input, { color: theme.text }]}
              placeholder="Type your message..."
              placeholderTextColor={theme.textSecondary}
              value={input}
              onChangeText={setInput}
              multiline
              editable={!isSending}
            />
            <ThemedView style={styles.actionsRow}>
              {isSending ? (
                <Pressable onPress={stop} style={styles.ghostButton}>
                  <ThemedText type="smallBold">Stop</ThemedText>
                </Pressable>
              ) : null}

              <Pressable
                onPress={handleSend}
                style={[
                  styles.sendButton,
                  {
                    backgroundColor:
                      input.trim().length > 0 && !isSending
                        ? theme.backgroundSelected
                        : theme.background,
                  },
                ]}>
                {isSending ? (
                  <ActivityIndicator />
                ) : (
                  <ThemedText type="smallBold">Send</ThemedText>
                )}
              </Pressable>
            </ThemedView>
          </ThemedView>
        </KeyboardAvoidingView>
      </SafeAreaView>
    </ThemedView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    flexDirection: 'row',
    justifyContent: 'center',
  },
  safeArea: {
    flex: 1,
    maxWidth: MaxContentWidth,
    paddingHorizontal: Spacing.three,
    gap: Spacing.three,
  },
  header: {
    gap: Spacing.one,
    paddingTop: Spacing.one,
  },
  scrollView: {
    flex: 1,
  },
  messagesContent: {
    gap: Spacing.two,
  },
  emptyState: {
    borderRadius: Spacing.three,
    padding: Spacing.three,
  },
  bubble: {
    borderRadius: Spacing.three,
    paddingHorizontal: Spacing.three,
    paddingVertical: Spacing.two,
    gap: Spacing.one,
  },
  userBubble: {
    marginLeft: Spacing.five,
  },
  assistantBubble: {
    marginRight: Spacing.five,
  },
  roleLabel: {
    opacity: 0.8,
  },
  messageText: {
    lineHeight: 22,
  },
  errorBox: {
    borderRadius: Spacing.three,
    padding: Spacing.three,
    gap: Spacing.one,
  },
  inputShell: {
    borderRadius: Spacing.three,
    padding: Spacing.two,
    gap: Spacing.two,
    marginBottom: Spacing.two,
  },
  input: {
    minHeight: 44,
    maxHeight: 140,
    fontSize: 16,
  },
  actionsRow: {
    flexDirection: 'row',
    justifyContent: 'flex-end',
    gap: Spacing.two,
    backgroundColor: 'transparent',
  },
  ghostButton: {
    paddingHorizontal: Spacing.three,
    paddingVertical: Spacing.two,
    borderRadius: Spacing.two,
  },
  sendButton: {
    paddingHorizontal: Spacing.three,
    paddingVertical: Spacing.two,
    borderRadius: Spacing.two,
    minWidth: 72,
    alignItems: 'center',
  },
});
