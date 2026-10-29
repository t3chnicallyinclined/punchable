// New bit (a form sheet from Bits "+", docs/design/HIG.md §3.13 as of 2026-10-29): the premise as he'd name it on a set
// list and a first line. Cancel leading, Add trailing (a write). The bit is created through POST /api/ideas (the same
// path as a typed jot: title = the premise, text = the line; the AI may attach it to a bit that already says this) and
// opened. Not the conversation: the agreed unit is a NAMED premise (docs/HANDOVER.md "organizing ideas").
import React, { useState } from 'react';
import { Platform, ScrollView, TextInput, View } from 'react-native';
import { Stack, router } from 'expo-router';
import { post } from '@/lib/api';
import { haptics } from '@/lib/haptics';
import { C, F } from '@/lib/theme';
import { Btn, SheetHead, T, useUI } from '@/lib/ui';

export default function NewBit() {
  const ui = useUI();
  const native = Platform.OS !== 'web';
  const [premise, setPremise] = useState('');
  const [line, setLine] = useState('');
  const [busy, setBusy] = useState(false);
  const close = () => (router.canGoBack() ? router.back() : router.navigate('/bits' as any));
  const can = !!premise.trim() && !busy;
  const add = async () => {
    if (!can) return;
    setBusy(true);
    try {
      const text = line.trim() || premise.trim();
      const r = await post<{ ok: boolean; id: string; attach?: { decision: string; into?: string; title?: string } }>('/api/ideas', { text, title: premise.trim(), reason: 'new bit (Bits +)' });
      haptics.success();
      const into = r.attach?.decision === 'attach' && r.attach.into ? r.attach.into : r.id;
      if (into !== r.id) ui.snack(`Filed under ${r.attach?.title || 'a bit that already says this'}`);
      close();
      router.push(`/joke/${encodeURIComponent(into)}` as any);
    } catch (e: any) {
      setBusy(false);
      ui.snack(`Couldn’t add it: ${e.message}`);
    }
  };
  const body = (
    <View style={{ gap: 14 }}>
      {!native && <SheetHead title="New bit" onCancel={close} onDone={can ? add : undefined} doneLabel="Add" testID="new-bit" />}
      <View style={{ gap: 6 }}>
        <T v="small">Premise · what you’d write on the set list</T>
        <TextInput testID="new-bit-premise" value={premise} onChangeText={setPremise} maxLength={60} autoFocus placeholder="Parking meters" placeholderTextColor={C.dim}
          returnKeyType="next" accessibilityLabel="Premise" clearButtonMode="while-editing"
          style={{ minHeight: 48, borderWidth: 1, borderColor: C.border, borderRadius: 10, paddingHorizontal: 12, color: C.text, fontFamily: F.bold, fontSize: 18 }} />
      </View>
      <View style={{ gap: 6 }}>
        <T v="small">First line · optional</T>
        <TextInput testID="new-bit-line" value={line} onChangeText={setLine} multiline placeholder="The line, the angle, the thing that happened" placeholderTextColor={C.dim}
          accessibilityLabel="First line" textAlignVertical="top"
          style={{ minHeight: 110, borderWidth: 1, borderColor: C.border, borderRadius: 10, paddingHorizontal: 12, paddingTop: 10, color: C.text, fontFamily: F.body, fontSize: 16, lineHeight: 22 }} />
      </View>
      {!native && <Btn testID="new-bit-add" v="primary" label={busy ? 'Adding…' : 'Add'} disabled={!can} onPress={add} />}
      <T v="small">It lands in Bits as its own premise. If a bit already says this, it files there instead and tells you.</T>
    </View>
  );
  if (!native) return <View style={{ flex: 1, backgroundColor: C.bg, padding: 16 }}>{body}</View>;
  return (
    <View testID="new-bit-sheet" style={{ flex: 1, backgroundColor: C.bg }}>
      <Stack.Screen options={{ title: 'New bit' }} />
      <Stack.Toolbar placement="left"><Stack.Toolbar.Button accessibilityLabel="Cancel" onPress={close}>Cancel</Stack.Toolbar.Button></Stack.Toolbar>
      <Stack.Toolbar placement="right"><Stack.Toolbar.Button variant="done" disabled={!can} accessibilityLabel="Add the bit" onPress={add}>{busy ? 'Adding…' : 'Add'}</Stack.Toolbar.Button></Stack.Toolbar>
      <ScrollView contentInsetAdjustmentBehavior="automatic" keyboardShouldPersistTaps="handled" contentContainerStyle={{ padding: 16, paddingBottom: 32 }}>{body}</ScrollView>
    </View>
  );
}
