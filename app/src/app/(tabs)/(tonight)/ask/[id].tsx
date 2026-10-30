// /ask/<id>: one conversation with his notebook (src/lib/ask.tsx).
import React from 'react';
import { useLocalSearchParams } from 'expo-router';
import { AskThreadScreen } from '@/lib/ask';

export default function AskChat() {
  const p = useLocalSearchParams<{ id: string }>();
  return <AskThreadScreen id={decodeURIComponent(String(p.id || ''))} />;
}
