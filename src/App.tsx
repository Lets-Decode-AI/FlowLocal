import Sidebar from './components/Sidebar';
import { ToastProvider } from './components/Toasts';
import { useHash } from './lib/hooks';
import History from './pages/History';
import Models from './pages/Models';
import Onboarding from './pages/Onboarding';
import Settings from './pages/Settings';

export default function App() {
  const route = useHash();
  return (
    <ToastProvider>
      {route === '/onboarding' ? (
        <>
          <div className="content-drag onb-drag" data-tauri-drag-region />
          <Onboarding />
        </>
      ) : (
        <div className="shell">
          <Sidebar route={route} />
          <main className="content">
            <div className="content-drag" data-tauri-drag-region />
            <div className="content-body">
              {route === '/models' ? <Models /> : route === '/settings' ? <Settings /> : <History />}
            </div>
          </main>
        </div>
      )}
    </ToastProvider>
  );
}
