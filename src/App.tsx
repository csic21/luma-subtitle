import { createHashRouter, RouterProvider } from "react-router-dom";

import { TooltipProvider } from "@/components/ui/tooltip";
import { I18nProvider } from "@/i18n";
import { AppLayout } from "@/layout/AppLayout";

// The data router supports blocking Back/navigation while subtitle edits are unsaved.
const router = createHashRouter([{ path: "*", element: <AppLayout /> }]);

export default function App() {
  return (
    <I18nProvider>
      <TooltipProvider delayDuration={180}>
        <RouterProvider router={router} />
      </TooltipProvider>
    </I18nProvider>
  );
}
