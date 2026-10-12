import { Component } from "react";
import { createRoot } from "react-dom/client";
import App from "./App.jsx";
import "./styles.css";

/** A render error shows a message instead of a blank page. */
class Fallback extends Component {
  state = { error: null };
  static getDerivedStateFromError(error) {
    return { error };
  }
  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="page">
        <div className="card error">
          The page failed to load: {this.state.error.message}. Your withdrawal keys are still stored in this browser. Reload, or open the page in another browser and import a downloaded key.
        </div>
      </div>
    );
  }
}

createRoot(document.getElementById("root")).render(
  <Fallback>
    <App />
  </Fallback>,
);
