extern crate fang_oost;
extern crate num_complex;
extern crate rayon;
extern crate cf_functions;
extern crate rand;
extern crate utils;
use utils::vec_to_mat;
use utils::vasicek;
use utils::risk_contributions;
extern crate cf_dist_utils;
use self::num_complex::Complex;
use self::rayon::prelude::*;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate serde_json;
use std::io;
use std::io::prelude::*; //needed for write
use std::io::BufReader;
use std::io::BufRead;
use std::fs::File;
#[macro_use]
#[cfg(test)]
extern crate approx;
#[macro_use]
#[cfg(test)]
extern crate itertools;
#[derive(Debug,Deserialize)]
struct Loan {
    balance:f64,
    pd:f64,
    lgd:f64,
    weight:Vec<f64>,
    #[serde(default = "default_num")]
    num:f64
}

fn default_num()->f64{
    1.0
}

#[derive(Debug,Deserialize)]
#[serde(rename_all = "camelCase")]
struct Parameters {
    lambda:f64,
    q:f64,
    alpha_l:f64,
    b_l:f64,
    sig_l:f64,
    t:f64,
    num_u:usize,
    x_min:f64,
    x_max:f64,
    num_x:Option<usize>,
    alpha:Vec<f64>,
    sigma:Vec<f64>,
    rho:Vec<f64>,
    y0:Vec<f64>
}

//lambda needs to be made negative, the probability of lambda occurring is
// -qX since X is negative.
fn get_liquidity_risk_fn(
    lambda:f64,
    q:f64
)->impl Fn(&Complex<f64>)->Complex<f64>
{
    move |u:&Complex<f64>|u-((-u*lambda).exp()-1.0)*q//-u
}

struct HoldDiscreteCF {
    cf: Vec<Complex<f64> >,
    num_w: usize //num columns
}

impl HoldDiscreteCF {
    pub fn new(num_u: usize, num_w: usize) -> HoldDiscreteCF{
        HoldDiscreteCF{
            cf: vec![Complex::new(0.0, 0.0); num_u*num_w],
            num_w //num rows
        }
    }
    #[cfg(test)]
    pub fn get_cf(&self)->&Vec<Complex<f64>>{
        return &self.cf
    }
    pub fn process_loan<U>(
        &mut self, loan: &Loan, 
        u_domain: &[Complex<f64>],
        log_lpm_cf: U
    ) where U: Fn(&Complex<f64>, &Loan)->Complex<f64>+std::marker::Sync+std::marker::Send
    {
        let vec_of_cf_u:Vec<Complex<f64>>=u_domain
            .par_iter()
            .map(|u|{
                log_lpm_cf(
                    &u, 
                    loan
                )
            }).collect(); 
        let num_w=self.num_w;
        self.cf.par_iter_mut().enumerate().for_each(|(index, elem)|{
            let row_num=vec_to_mat::get_row_from_index(index, num_w);
            let col_num=vec_to_mat::get_col_from_index(index, num_w);
            *elem+=vec_of_cf_u[col_num]*loan.weight[row_num]*loan.num;
        });
    }
    pub fn experiment_loan<U>(
        &self, loan: &Loan, 
        u_domain: &[Complex<f64>],
        log_lpm_cf: U
    )->Vec<Complex<f64>> where U: Fn(&Complex<f64>, &Loan)->Complex<f64>+std::marker::Sync+std::marker::Send
    {
        let vec_of_cf_u:Vec<Complex<f64>>=u_domain
            .par_iter()
            .map(|u|{
                log_lpm_cf(
                    &u, 
                    loan
                )
            }).collect(); 
        let num_w=self.num_w;
        self.cf.par_iter().enumerate().map(|(index, elem)|{
            let row_num=vec_to_mat::get_row_from_index(index, num_w);
            let col_num=vec_to_mat::get_col_from_index(index, num_w);
            elem+vec_of_cf_u[col_num]*loan.weight[row_num]*loan.num
        }).collect::<Vec<_>>()
    }
    pub fn get_full_cf<U>(&self, mgf:U)->Vec<Complex<f64>>
    where U: Fn(&[Complex<f64>])->Complex<f64>+std::marker::Sync+std::marker::Send
    {
        self.cf.par_chunks(self.num_w)
            .map(mgf).collect()
    }
}

fn get_log_lpm_cf<T, U>(
    lgd_cf:T,
    liquidity_cf:U
)-> impl Fn(&Complex<f64>, &Loan)->Complex<f64>
    where T: Fn(&Complex<f64>, f64)->Complex<f64>,
          U: Fn(&Complex<f64>)->Complex<f64>
{
    move |u:&Complex<f64>, loan:&Loan|{
        (lgd_cf(&liquidity_cf(u), loan.lgd*loan.balance)-1.0)*loan.pd
    }
}
fn get_lgd_cf_fn(
    speed:f64,
    long_run_average:f64,
    sigma:f64,
    t:f64,
    x0:f64
)->impl Fn(&Complex<f64>, f64)->Complex<f64>{
    move |u:&Complex<f64>, l:f64|{
        //while "l" should be negative, note that the cir_mgf makes the "u" as negative 
        //since its derived from the CIR model which is a discounting model.
        //In general, implementations should make l negative when input into the 
        //lgd_cf
        cf_functions::cir_mgf(
            &(u*l), speed, long_run_average*speed, 
            sigma, t, x0
        )
    }   
}
#[cfg(test)]
fn test_mgf(u_weights:&[Complex<f64>])->Complex<f64>{
    u_weights.iter()
        .sum::<Complex<f64>>().exp()
}

fn gamma_mgf(variance:Vec<f64>)->
   impl Fn(&[Complex<f64>])->Complex<f64>
{
    move |u_weights:&[Complex<f64>]|->Complex<f64>{
        u_weights.iter().zip(&variance).map(|(u, v)|{
            -(1.0-v*u).ln()/v
        }).sum::<Complex<f64>>().exp()
    }
}

//inefficient for large portfolios.  dont use
#[cfg(test)]
pub fn portfolio_expectation(
    pd:&[f64],
    expectation_l:&[f64],
    balance:&[f64]
)->f64{
    izip!(pd, expectation_l, balance)
        .map(|(p, el, b)|{
            p*el*b
        }).sum()
}

//inefficient for large portfolios.  dont use
#[cfg(test)]
pub fn portfolio_variance(
    pd:&[f64],
    expectation_l:&[f64], //lgd
    variance_l:&[f64],
    balance:&[f64],
    weights:&[Vec<f64>], //n by m
    variance_systemic:&[f64] //length m, assumed independent since a vector
)->f64{

    let vel=izip!(
        pd, balance,
        variance_l
    ).map(|(p, b, vl)|{
        p*b.powi(2)*vl
    }).sum::<f64>();
    
    let evl=variance_systemic.iter().enumerate().map(|(index, v)|{
        izip!(pd, expectation_l, weights, balance).map(|(p, el, w,b)|{
            p*el*b*w[index]
        }).sum::<f64>().powi(2)*v
    }).sum::<f64>();
    vel+evl
}
#[cfg(test)]
pub fn portfolio_variance_from_cf(
    pd:&[f64],
    expectation_l:&[f64], //lgd
    variance_l:&[f64],
    balance:&[f64],
    weights:&[Vec<f64>], //n by m
    variance_systemic:&[f64] //length m, assumed independent since a vector
)->f64{

    let vel=izip!(
        pd, balance,
        expectation_l,
        variance_l
    ).map(|(p, b, e_l, vl)|{
        p*b.powi(2)*(vl+e_l.powi(2))
    }).sum::<f64>();
    
    let el=portfolio_expectation(pd, expectation_l, balance);

    let evl=variance_systemic.iter().enumerate().map(|(index, v)|{
        izip!(pd, expectation_l, weights, balance).map(|(p, e_l, w,b)|{
            p*e_l*b*w[index]
        }).sum::<f64>().powi(2)*(v+1.0)
    }).sum::<f64>();
    vel+evl-el.powi(2)
}


fn risk_contribution_existing_loan(
    loan:&Loan, gamma_variances:&[f64], risk_measure:f64,
    variance_l:f64,
    expectation_portfolio:f64, 
    variance_portfolio:f64,
    lambda:f64, q:f64
)->f64{
    let expectation_liquid=risk_contributions::expectation_liquidity(
        lambda, q, expectation_portfolio
    );
    let variance_liquid=risk_contributions::variance_liquidity(
        lambda, q, expectation_portfolio, variance_portfolio
    );
    let rj=0.0;
    risk_contributions::generic_risk_contribution(
        loan.pd, loan.lgd*loan.balance, 
        variance_l, expectation_portfolio, 
        variance_portfolio, 
        risk_contributions::variance_from_independence(&loan.weight, gamma_variances),
        risk_contributions::scale_contributions(
            risk_measure, expectation_liquid, variance_liquid
        ),
        rj,
        loan.balance,
        lambda, 
        lambda,
        q
    )
}

fn main()-> Result<(), io::Error> {
    let args: Vec<String> = std::env::args().collect();
    let Parameters{
        lambda, q, alpha_l, 
        b_l, sig_l, t, num_u, 
        x_min, x_max, alpha, 
        sigma, rho, y0, ..
    }= serde_json::from_str(args[1].as_str())?;
    let num_w=alpha.len();
    let liquid_fn=get_liquidity_risk_fn(lambda, q);
    let lgd_fn=get_lgd_cf_fn(alpha_l, b_l, sig_l, t, b_l);//assumption is that it starts at the lgd mean...
    let log_lpm_cf=get_log_lpm_cf(&lgd_fn, &liquid_fn);

    let mut discrete_cf=HoldDiscreteCF::new(
        num_u, num_w
    );

    let u_domain:Vec<Complex<f64>>=fang_oost::get_u_domain(
        num_u, x_min, x_max
    ).collect();

    let f = File::open(args[2].as_str())?;
    let file = BufReader::new(&f);
    for line in file.lines() {
        let loan: Loan = serde_json::from_str(&line?)?;
        discrete_cf.process_loan(&loan, &u_domain, &log_lpm_cf);
    }  
    
    let expectation=vasicek::compute_integral_expectation_long_run_one(
        &y0, &alpha, t
    );
    let variance=vasicek::compute_integral_variance(
        &alpha, &sigma, 
        &rho, t
    );

    let v_mgf=vasicek::get_vasicek_mgf(expectation, variance);
    let final_cf:Vec<Complex<f64>>=discrete_cf.get_full_cf(&v_mgf);
    if args.len()>3 {
        let x_domain:Vec<f64>=fang_oost::get_x_domain(1024, x_min, x_max).collect();
        let density:Vec<f64>=fang_oost::get_density(
            x_min, x_max, 
            fang_oost::get_x_domain(1024, x_min, x_max), 
            &final_cf
        ).collect();
        let json_results=json!({"x":x_domain, "density":density});
        let mut file_w = File::create(args[3].as_str())?;
        file_w.write_all(json_results.to_string().as_bytes())?;
    }
    

    let max_iterations=100;
    let tolerance=0.0001;
    let (es, var)=cf_dist_utils::get_expected_shortfall_and_value_at_risk_discrete_cf(
        0.01, 
        x_min,
        x_max,
        max_iterations,
        tolerance,
        &final_cf
    );
    let variance_portfolio=cf_dist_utils::get_variance_discrete_cf(
        x_min, x_max, &final_cf
    );
    let expectation_portfolio=cf_dist_utils::get_expectation_discrete_cf(
        x_min, x_max, &final_cf
    );
    println!("This is ES: {}", es);
    println!("This is VaR: {}", var);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn construct_hold_discrete_cf(){
        let discrete_cf=HoldDiscreteCF::new(
            256, 3
        );
        let cf=discrete_cf.get_cf();
        assert_eq!(cf.len(), 256*3);
        assert_eq!(cf[0], Complex::new(0.0, 0.0)); //first three should be the same "u"
        assert_eq!(cf[1], Complex::new(0.0, 0.0));
        assert_eq!(cf[2], Complex::new(0.0, 0.0));
    }
    #[test]
    fn test_process_loan(){
        let mut discrete_cf=HoldDiscreteCF::new(
            256, 3
        );
        let loan=Loan{
            pd:0.05,
            lgd:0.5,
            balance:1000.0,
            weight:vec![0.5, 0.5, 0.5],
            num:1.0
        };
        let log_lpm_cf=|_u:&Complex<f64>, _loan:&Loan|{
            Complex::new(1.0, 0.0)
        };
        let u_domain:Vec<Complex<f64>>=fang_oost::get_u_domain(
            256, 0.0, 1.0
        ).collect();
        discrete_cf.process_loan(&loan, &u_domain, &log_lpm_cf);
        let cf=discrete_cf.get_cf();
        assert_eq!(cf.len(), 256*3);
        cf.iter().for_each(|cf_el|{
            assert_eq!(cf_el, &Complex::new(0.5 as f64, 0.0 as f64));
        });
        
    }
    #[test]
    fn test_process_loans_with_final(){
        let mut discrete_cf=HoldDiscreteCF::new(
            256, 3
        );
        let loan=Loan{
            pd:0.05,
            lgd:0.5,
            balance:1000.0,
            weight:vec![0.5, 0.5, 0.5],
            num:1.0
        };
        let u_domain:Vec<Complex<f64>>=fang_oost::get_u_domain(
            256, 0.0, 1.0
        ).collect();
        let log_lpm_cf=|_u:&Complex<f64>, _loan:&Loan|{
            Complex::new(1.0, 0.0)
        };
        discrete_cf.process_loan(&loan, &u_domain, &log_lpm_cf);
        let final_cf:Vec<Complex<f64>>=discrete_cf.get_full_cf(&test_mgf);
    
        assert_eq!(final_cf.len(), 256);
        final_cf.iter().for_each(|cf_el|{
            assert_eq!(cf_el, &Complex::new(1.5 as f64, 0.0 as f64).exp());
        });
    }
    #[test]
    fn test_actually_get_density(){
        let x_min=-6000.0;
        let x_max=0.0;
        let mut discrete_cf=HoldDiscreteCF::new(
            256, 1
        );
        let lambda=1000.0;
        let q=0.0001;
        let liquid_fn=get_liquidity_risk_fn(lambda, q);

        let t=1.0;
        let alpha_l=0.2;
        let b_l=1.0;
        let sig_l=0.2;
        let lgd_fn=get_lgd_cf_fn(alpha_l, b_l, sig_l, t, b_l);//assumption is that it starts at the lgd mean...
        let u_domain:Vec<Complex<f64>>=fang_oost::get_u_domain(
            256, x_min, x_max
        ).collect();
        let log_lpm_cf=get_log_lpm_cf(&lgd_fn, &liquid_fn);

        let loan=Loan{
            pd:0.05,
            lgd:0.5,
            balance:1.0,
            weight:vec![1.0],
            num:10000.0
        };
        discrete_cf.process_loan(&loan, &u_domain, &log_lpm_cf);
        let y0=vec![1.0];
        let alpha=vec![0.3];
        let sigma=vec![0.3];
        let rho=vec![1.0];
        let t=1.0;
        let expectation=vasicek::compute_integral_expectation_long_run_one(
            &y0, &alpha, t
        );
        let variance=vasicek::compute_integral_variance(
            &alpha, &sigma, 
            &rho, t
        );

        let v_mgf=vasicek::get_vasicek_mgf(expectation, variance);
        
        let final_cf:Vec<Complex<f64>>=discrete_cf.get_full_cf(&v_mgf);

        assert_eq!(final_cf.len(), 256);
        let max_iterations=100;
        let tolerance=0.0001;
        let (
            es, 
            var
        )=cf_dist_utils::get_expected_shortfall_and_value_at_risk_discrete_cf(
            0.01, 
            x_min,
            x_max,
            max_iterations,
            tolerance,
            &final_cf
        );
        println!("this is es: {}", es);
        println!("this is var: {}", var);
        assert!(es>var);
    }
    #[test]
    fn test_gamma_cf(){
        let kappa=2.0;
        //let theta=0.5;
        let u=Complex::new(0.5, 0.5);
        let theta=0.5;
        let cf=gamma_mgf(vec![theta]);
        let result=cf(&vec![u]);
        let expected=(1.0-u*theta).powf(-kappa);
        assert_eq!(result, expected);
    }
    #[test]
    fn test_compare_expected_value(){
        let balance=1.0;
        let pd=0.05;
        let lgd=0.5;
        let num_loans=10000.0;
        let lambda=1000.0; //loss in the event of a liquidity crisis
        let q=0.01/(num_loans*pd*lgd*balance);
        let expectation=-pd*lgd*balance*(1.0+lambda*q)*num_loans;
        let x_min=(expectation-lambda)*3.0;
        let x_max=0.0;
        let num_u:usize=1024;
        let mut discrete_cf=HoldDiscreteCF::new(
            num_u, 1
        );
       
        let liquid_fn=get_liquidity_risk_fn(lambda, q);

        //the exponent is negative because l represents a loss
        let lgd_fn=|u:&Complex<f64>, l:f64|(-u*l).exp();
        
        let u_domain:Vec<Complex<f64>>=fang_oost::get_u_domain(
            num_u, x_min, x_max
        ).collect();
        let log_lpm_cf=get_log_lpm_cf(&lgd_fn, &liquid_fn);
        
        let loan=Loan{
            pd,
            lgd,
            balance,
            weight:vec![1.0],
            num:num_loans//homogenous
        };
        discrete_cf.process_loan(&loan, &u_domain, &log_lpm_cf);
        let v=vec![0.3];
        let v_mgf=gamma_mgf(v);        
        let final_cf:Vec<Complex<f64>>=discrete_cf.get_full_cf(&v_mgf);
        assert_eq!(final_cf.len(), num_u);
        let expectation_approx=cf_dist_utils::get_expectation_discrete_cf(x_min, x_max, &final_cf);
        
        assert_abs_diff_eq!(expectation_approx, expectation, epsilon=0.00001);
    }
    #[test]
    fn test_compare_expected_value_and_variance(){
        let balance=1.0;
        let pd=0.05;
        let lgd=0.5;
        let num_loans=10000.0;
        let lambda=1000.0; //loss in the event of a liquidity crisis
        let q=0.01/(num_loans*pd*lgd*balance);
        let expectation=-portfolio_expectation(
            &vec![pd; num_loans as usize],
            &vec![lgd; num_loans as usize],
            &vec![balance; num_loans as usize]
        );
        let lgd_variance=0.2;
        let v1=vec![0.4, 0.3];
        let v2=vec![0.4, 0.3];

        let v_mgf=gamma_mgf(v1); 
        let weight1=vec![0.4, 0.6];
        let weight2=vec![0.4, 0.6];
        let weight3=vec![0.4, 0.6];
        let variance=portfolio_variance(
            &vec![pd; num_loans as usize],
            &vec![lgd; num_loans as usize],
            &vec![lgd_variance; num_loans as usize],
            &vec![balance; num_loans as usize],
            &vec![weight1; num_loans as usize],
            &v2
        );
        let variance_cf=portfolio_variance_from_cf(
            &vec![pd; num_loans as usize],
            &vec![lgd; num_loans as usize],
            &vec![lgd_variance; num_loans as usize],
            &vec![balance; num_loans as usize],
            &vec![weight2; num_loans as usize],
            &v2
        );

        println!("norma variance: {}", variance);
        println!("cf variance: {}", variance_cf);

        let expectation_liquid=risk_contributions::expectation_liquidity(
            lambda, q, expectation
        );
        let variance_liquid=risk_contributions::variance_liquidity(
            lambda, q, expectation, variance
        );
        let variance_liquid_cf=risk_contributions::variance_liquidity(
            lambda, q, expectation, variance_cf
        );

        let x_min=(expectation-lambda)*3.0;
        let x_max=0.0;
        let num_u:usize=1024;
        let mut discrete_cf=HoldDiscreteCF::new(
            num_u, v2.len()
        );
       
        let liquid_fn=get_liquidity_risk_fn(lambda, q);

        //the exponent is negative because l represents a loss
        let a=1.0/lgd_variance;
        let lgd_fn=|u:&Complex<f64>, l:f64|cf_functions::gamma_cf(
            &(-u*l), a, lgd_variance
        );
        
        let u_domain:Vec<Complex<f64>>=fang_oost::get_u_domain(
            num_u, x_min, x_max
        ).collect();
        let log_lpm_cf=get_log_lpm_cf(&lgd_fn, &liquid_fn);
        
        let loan=Loan{
            pd,
            lgd,
            balance,
            weight:weight3,
            num:num_loans//homogenous
        };
        discrete_cf.process_loan(&loan, &u_domain, &log_lpm_cf);
           
        let final_cf:Vec<Complex<f64>>=discrete_cf.get_full_cf(&v_mgf);
        assert_eq!(final_cf.len(), num_u);
        let expectation_approx=cf_dist_utils::get_expectation_discrete_cf(
            x_min, x_max, &final_cf
        );
        let variance_approx=cf_dist_utils::get_variance_discrete_cf(
            x_min, x_max, &final_cf
        );
        
        assert_abs_diff_eq!(expectation_approx, expectation_liquid, epsilon=0.00001);
        //this seems to be awfully large variation.  Is it a problem with the approximation or the variance computation?
        assert_abs_diff_eq!((variance_approx-variance_liquid)/variance_liquid, 0.0, epsilon=0.01);
        assert_abs_diff_eq!(variance_approx, variance_liquid_cf, epsilon=0.01);
    }
}