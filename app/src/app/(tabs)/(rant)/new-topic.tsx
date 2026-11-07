// New topic (SPEC 2026-11-06 §12.2): "+" works like Bits' "+" (a page, not a form): one focused field at Title 1,
// "What's the rant about?". Return makes the topic and this page becomes the topic, the search already run. Back with
// nothing typed makes nothing. Typed (jots and topics are typed). Request: POST /api/topics.
import React, { useState } from 'react';
import { Text, TextInput } from 'react-native';
import { router } from 'expo-router';
import { ApiError, useOnline } from '@/lib/api';
import { rant } from '@/lib/rant';
import { haptics } from '@/lib/haptics';
import { Screen } from '@/lib/Screen';
import { C, F } from '@/lib/theme';
import { T, useUI } from '@/lib/ui';

export default function NewTopic() {
  const ui = useUI();
  const online = useOnline();
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const make = async () => {
    const t = text.trim();
    if (!t || busy) return;
    setBusy(true);
    try {
      const r = await rant.addTopic(t);
      haptics.success();
      router.replace({ pathname: '/topic/[topic]', params: { topic: r.topic.id } } as any);
    } catch (e: any) {
      setBusy(false);
      ui.snack(e instanceof ApiError ? `Couldn’t add it: ${e.message}` : 'Can’t reach Punchable. Nothing changed.');
    }
  };
  return (
    <Screen title="" testID="topic-new" backLabel="Rant">
      <TextInput testID="topic-new-input" value={text} onChangeText={setText} autoFocus placeholder="What’s the rant about?" placeholderTextColor={C.dim}
        accessibilityLabel="What’s the rant about?" returnKeyType="go" submitBehavior="submit" onSubmitEditing={() => void make()} maxLength={200}
        editable={!busy} maxFontSizeMultiplier={1.8} style={{ minHeight: 44, marginTop: 8, fontFamily: F.g7, fontSize: 28, lineHeight: 34, letterSpacing: -0.6, color: C.text }} />
      <T v="small">{!online ? 'Offline: a topic needs a signal to save.' : busy ? 'Opening…' : 'Return opens it.'}</T>
      <Text style={{ height: 0 }} />
    </Screen>
  );
}
