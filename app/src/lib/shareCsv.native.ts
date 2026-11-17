// A spreadsheet file handed to the person, on the phone: written to the app's cache folder as `name` and offered
// through the Share sheet as a file (Save to Files, Mail, Numbers), then deleted. The web twin is shareCsv.ts.
// Same file API as lib/draftFiles.native.ts.
import * as FS from 'expo-file-system/legacy';
import { Share } from 'react-native';

export async function shareCsv(name: string, csv: string): Promise<'downloaded' | 'shared' | 'failed'> {
  const safe = name.replace(/[^A-Za-z0-9._-]/g, '-');
  const uri = `${FS.cacheDirectory}${safe}`;
  try {
    await FS.writeAsStringAsync(uri, csv);
    await Share.share({ url: uri });
    return 'shared';
  } catch {
    return 'failed';
  } finally {
    FS.deleteAsync(uri, { idempotent: true }).catch(() => {});
  }
}
