import React from "react";
import ReactDOM from "react-dom/client";
import "../index.css";
import PetWindow from "./PetWindow";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
        <PetWindow />
    </React.StrictMode>
);
