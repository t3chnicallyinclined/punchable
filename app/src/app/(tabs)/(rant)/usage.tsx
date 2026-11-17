// Settings › Usage (SPEC 2026-11-17 §13, phase 1): what Punchable's AI cost this month and how much he used it, from
// GET /api/admin/usage (owner only on the server, pinned to his Keycloak subject). Read-only. Every word and number
// on it comes from rules/usageRules.ts (tested); this file only lays it out. Counts and money only: nothing he wrote.
// Reviewed against the HIG by the iOS design expert (2026-11-17): the figure first, grouped rows, Try again on a
// failure, the export as a real file through the Share sheet, at the bottom within reach.
import React from 'react';
import { Platform, View } from 'react-native';
import { useApi } from '@/lib/api';
import { Icon } from '@/lib/Icon';
import { shareCsv } from '@/lib/shareCsv';
import { C } from '@/lib/theme';
import { Btn, Card, List, ListRow, Skeleton, T, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { STALE, UsageReport, csvName, usageCsv, usageError, usageView } from '@/rules/usageRules';

export default function Usage() {
  const ui = useUI();
  const api = useApi<UsageReport>('/api/admin/usage', { cache: false });
  const v = api.data ? usageView(api.data) : null;
  const failed = !api.loading && !!api.rawError;

  const exportCsv = async () => {
    if (!api.data) return;
    const r = await shareCsv(csvName(api.data), usageCsv(api.data));
    if (r === 'downloaded') ui.snack('Spreadsheet downloaded');
    else if (r === 'failed') ui.snack('Couldn’t make the spreadsheet');
  };

  return (
    <Screen title="Usage" back testID="usage" onRefresh={api.reload}>
      {!v ? (
        <Card testID="usage-status" style={{ gap: 12 }}>
          {api.loading ? <Skeleton lines={3} testID="usage-loading" /> : <T testID="usage-line">{usageError(api.rawError)}</T>}
          {failed && <Btn small testID="usage-retry" label="Try again" onPress={api.reload} />}
        </Card>
      ) : (
        <>
          {failed && (
            <View style={{ flexDirection: 'row', alignItems: 'center', gap: 12 }} testID="usage-stale">
              <T v="small" style={{ flex: 1 }}>{STALE}</T>
              <Btn small label="Try again" onPress={api.reload} />
            </View>
          )}
          <Card style={{ gap: 4 }} testID="usage-month">
            <T v="small" accessibilityRole="header">{v.heading}</T>
            <T v="big" testID="usage-spent" accessibilityLabel={v.spentLabel}>{v.spent}</T>
            <T v="mut" testID="usage-pace">{v.pace}</T>
            <T v="mut" testID="usage-active">{v.active}</T>
            {v.perDay && <T v="mut" testID="usage-per-day">{v.perDay}</T>}
          </Card>

          {v.kinds.length > 0 && (
            <Card style={{ gap: 4 }} testID="usage-kinds">
              <T v="eb" accessibilityRole="header">What it cost</T>
              <List>
                {v.kinds.map((k) => (
                  <ListRow key={k.key} testID={`usage-kind-${k.key}`} title={k.name} sub={k.detail} chevron={false}
                    right={<T v="num">{k.usd}</T>} />
                ))}
              </List>
            </Card>
          )}

          {v.cover && (
            <Card style={{ gap: 4 }} testID="usage-cover">
              <T v="eb" accessibilityRole="header">If you charged for it</T>
              <T testID="usage-cover-line">{v.cover.line}</T>
              <T v="small">{v.cover.note}</T>
            </Card>
          )}

          <Card style={{ gap: 4 }} testID="usage-activity">
            <T v="eb" accessibilityRole="header">{v.activityHeading}</T>
            <List>
              {v.activity.map((a) => (
                <ListRow key={a.key} testID={`usage-${a.key}`} title={a.label} sub={a.sub} chevron={false} right={<T v="num">{a.value}</T>} />
              ))}
            </List>
          </Card>

          <Btn testID="usage-export" label={Platform.OS === 'web' ? 'Download as a spreadsheet' : 'Share as a spreadsheet'}
            icon={<Icon name="upload" size={18} color={C.text} />} onPress={exportCsv} />
          <View>
            <T v="small" testID="usage-footnote">{v.footnote}</T>
          </View>
        </>
      )}
    </Screen>
  );
}
