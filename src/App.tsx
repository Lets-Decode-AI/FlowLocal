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
        <Onboarding />
      ) : (
        <div className="shell">
          <Sidebar route={route} />
          <main className="content">
            {route === '/models' ? <Models /> : route === '/settings' ? <Settings /> : <History />}
          </main>
        </div>
      )}
    </ToastProvider>
  );
}
