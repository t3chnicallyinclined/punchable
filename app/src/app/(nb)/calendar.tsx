// The calendar is dropped (SIMPLE.md): the dated Shows list is the calendar.
import { Redirect } from 'expo-router';

export default function CalendarRedirect() {
  return <Redirect href={'/shows' as any} />;
}
