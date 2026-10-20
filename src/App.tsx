import { createHashRouter, RouterProvider } from "react-router-dom";
import { IntroProvider } from "./app/intro";
import { OperationsProvider } from "./app/operations";
import { PreferencesProvider } from "./app/preferences";
import { ToastProvider } from "./app/toasts";
import { AppShell } from "./components/shell/AppShell";
import { AiWorkspacePage } from "./pages/AiWorkspacePage";
import { AutomationPage } from "./pages/AutomationPage";
import { DashboardPage } from "./pages/DashboardPage";
import { HomeworkDetailPage } from "./pages/HomeworkDetailPage";
import { HomeworkListPage } from "./pages/HomeworkListPage";
import { InboxPage } from "./pages/InboxPage";
import { NotFoundPage } from "./pages/NotFoundPage";
import { SettingsPage } from "./pages/SettingsPage";
import { WeekIndexRedirect, WeekPage } from "./pages/WeekPage";

// Hash-router: стабильно работает внутри Tauri (tauri://) без настройки сервера.
const router = createHashRouter([
  {
    element: <AppShell />,
    children: [
      { index: true, element: <DashboardPage /> },
      { path: "inbox", element: <InboxPage /> },
      { path: "week", element: <WeekIndexRedirect /> },
      { path: "subjects/:subjectId/week/:n", element: <WeekPage /> },
      { path: "homework", element: <HomeworkListPage /> },
      { path: "homework/:homeworkId", element: <HomeworkDetailPage /> },
      { path: "ai", element: <AiWorkspacePage /> },
      { path: "automation", element: <AutomationPage /> },
      { path: "settings", element: <SettingsPage /> },
      { path: "*", element: <NotFoundPage /> },
    ],
  },
]);

export function App() {
  return (
    <PreferencesProvider>
      <ToastProvider>
        <OperationsProvider>
          <IntroProvider>
            <RouterProvider router={router} />
          </IntroProvider>
        </OperationsProvider>
      </ToastProvider>
    </PreferencesProvider>
  );
}
