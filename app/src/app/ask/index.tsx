// /ask (docs/API_CONTRACT.md "Ask"): a new question, then his past conversations. /ask?q=… asks at once (the Notebook
// search box sends a question here). Not a tab: the Ask button sits in the header of Write, Boards and Notebook.
import React from 'react';
import { useLocalSearchParams } from 'expo-router';
import { AskHome } from '@/lib/ask';
import { Screen } from '@/lib/ui';

export default function AskIndex() {
  const p = useLocalSearchParams<{ q?: string }>();
  const q = typeof p.q === 'string' ? p.q.trim().slice(0, 1000) : '';
  return (
    <Screen title="Ask" back testID="ask" maxWidth={760}>
      <AskHome first={q || undefined} />
    </Screen>
  );
}
