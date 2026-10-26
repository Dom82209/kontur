import { lazy, Suspense, type ComponentType, type ReactNode } from "react";
import { createHashRouter, RouterProvider } from "react-router-dom";
import { IntroProvider } from "./app/intro";
import { OperationsProvider } from "./app/operations";
import { PreferencesProvider } from "./app/preferences";
import { ToastProvider } from "./app/toasts";
import { AppShell } from "./components/shell/AppShell";
import { SkeletonRows } from "./components/ui/States";

function page<T extends Record<string, ComponentType>>(loader: () => Promise<T>, name: keyof T) {
  return lazy(async () => ({ default: (await loader())[name] }));
}

const AiWorkspacePage = page(() => import("./pages/AiWorkspacePage"), "AiWorkspacePage");
const AutomationPage = page(() => import("./pages/AutomationPage"), "AutomationPage");
const DashboardPage = page(() => import("./pages/DashboardPage"), "DashboardPage");
const HomeworkDetailPage = page(() => import("./pages/HomeworkDetailPage"), "HomeworkDetailPage");
const HomeworkListPage = page(() => import("./pages/HomeworkListPage"), "HomeworkListPage");
const InboxPage = page(() => import("./pages/InboxPage"), "InboxPage");
const MaterialsPage = page(() => import("./pages/MaterialsPage"), "MaterialsPage");
const NotFoundPage = page(() => import("./pages/NotFoundPage"), "NotFoundPage");
const SettingsPage = page(() => import("./pages/SettingsPage"), "SettingsPage");
const WeekIndexRedirect = page(() => import("./pages/WeekPage"), "WeekIndexRedirect");
const WeekPage = page(() => import("./pages/WeekPage"), "WeekPage");

function deferred(element: ReactNode) {
  return <Suspense fallback={<main className="page" aria-busy="true"><SkeletonRows count={6} /></main>}>{element}</Suspense>;
}

// Hash-router: стабильно работает внутри Tauri (tauri://) без настройки сервера.
const router = createHashRouter([
  {
    element: <AppShell />,
    children: [
      { index: true, element: deferred(<DashboardPage />) },
      { path: "inbox", element: deferred(<InboxPage />) },
      { path: "materials", element: deferred(<MaterialsPage />) },
      { path: "week", element: deferred(<WeekIndexRedirect />) },
      { path: "subjects/:subjectId/week/:n", element: deferred(<WeekPage />) },
      { path: "homework", element: deferred(<HomeworkListPage />) },
      { path: "homework/:homeworkId", element: deferred(<HomeworkDetailPage />) },
      { path: "ai", element: deferred(<AiWorkspacePage />) },
      { path: "automation", element: deferred(<AutomationPage />) },
      { path: "settings", element: deferred(<SettingsPage />) },
      { path: "*", element: deferred(<NotFoundPage />) },
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
