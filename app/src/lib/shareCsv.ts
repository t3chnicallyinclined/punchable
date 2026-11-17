// A spreadsheet file handed to the person, on the web: a download of `name` (the phone's twin, shareCsv.native.ts,
// writes the file and opens the Share sheet on it). Resolves with what happened, for the snack.
export async function shareCsv(name: string, csv: string): Promise<'downloaded' | 'shared' | 'failed'> {
  try {
    const href = URL.createObjectURL(new Blob([csv], { type: 'text/csv' }));
    const a = document.createElement('a');
    a.href = href;
    a.download = name;
    a.click();
    setTimeout(() => URL.revokeObjectURL(href), 0);
    return 'downloaded';
  } catch {
    return 'failed';
  }
}
