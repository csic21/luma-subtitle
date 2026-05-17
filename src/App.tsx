import { HashRouter } from "react-router-dom";

import { TooltipProvider } from "@/components/ui/tooltip";
import { I18nProvider } from "@/i18n";
import { AppLayout } from "@/layout/AppLayout";

export default function App() {
  return (
    <I18nProvider>
      <TooltipProvider delayDuration={180}>
        <HashRouter>
          <AppLayout />
        </HashRouter>
      </TooltipProvider>
    </I18nProvider>
  );
}
