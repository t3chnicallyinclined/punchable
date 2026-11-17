// /write was an older "Write" home with no way in (SPEC 2026-11-16 §2.3). A bit is written on its own page now
// (joke/[id]: the bit is a document), so the URL is kept and opens Bits, where "+" starts a new one.
import React from 'react';
import { Redirect } from 'expo-router';

export default function Write() {
  return <Redirect href="/bits" />;
}
