use leptos::*;

#[component]
fn Header() -> impl IntoView {
    view! {
        <header>
            <h1>
                <a href="/">"Andrea Soprani"</a>
            </h1>
            <img src="/assets/propic.jpg" alt="Andrea Soprani"/>
            <p>This is me.</p>
            <p class="view">
                <a href="https://github.com/andreasoprani">"View My GitHub Profile"</a>
            </p>
        </header>
    }
}

#[component]
fn Bio() -> impl IntoView {
    view! {
        <section>
            <h2 id="personal-info">"Personal info"</h2>
            <p>
                "Born in Parma, Italy." <br/> "Currently living in Milan, Italy." <br/>
                "MSc in Computer Science and Engineering @ "
                <a href="https://polimi.it">"Politecnico di Milano"</a> "." <br/>
                "Working as a Software Engineer @ "
                <a href="https://quickalgorithm.com">"Quick Algorithm"</a> "."
            </p>
        </section>
    }
}

#[component]
fn Footer() -> impl IntoView {
    view! {
        <footer>
            <p>
                <small>
                    "Built with " <a href="https://leptos.dev">"Leptos"</a> "." <br/> "Hosted on "
                    <a href="https://pages.github.com/">"Github Pages"</a> "." <br/>
                    "Based on the GH Pages "
                    <a href="https://github.com/orderedlist/minimal">"minimal"</a> " theme by "
                    <a href="https://github.com/orderedlist">"orderedlist"</a> "."
                </small>
            </p>
        </footer>
    }
}

#[component]
fn Homepage() -> impl IntoView {
    view! {
        <div class="wrapper">
            <Header/>
            <Bio/>
            <Footer/>
        </div>
    }
}

fn main() {
    mount_to_body(|| {
        view! { <Homepage/> }
    })
}
